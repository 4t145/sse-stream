use std::{
    collections::VecDeque,
    num::ParseIntError,
    str::Utf8Error,
    task::{ready, Context, Poll},
};

use crate::Sse;
use bytes::Buf;
use futures_util::{stream::MapOk, Stream, TryStreamExt};
use http_body::{Body, Frame};
use http_body_util::{BodyDataStream, StreamBody};

const BOM_HEADER: &[u8] = b"\xEF\xBB\xBF";

struct ParserState {
    parsed: VecDeque<Sse>,
    current: Option<Sse>,
    unfinished_line: Vec<u8>,
    skip_leading_lf: bool,
    first_line: bool,
}

impl Default for ParserState {
    fn default() -> Self {
        Self {
            parsed: VecDeque::new(),
            current: None,
            unfinished_line: Vec::new(),
            skip_leading_lf: false,
            first_line: true,
        }
    }
}

pin_project_lite::pin_project! {
    pub struct SseStream<B: Body> {
        #[pin]
        body: BodyDataStream<B>,
        parser: ParserState,
    }
}

pub type ByteStreamBody<S, D> = StreamBody<MapOk<S, fn(D) -> Frame<D>>>;
impl<E, S, D> SseStream<ByteStreamBody<S, D>>
where
    S: Stream<Item = Result<D, E>>,
    E: std::error::Error,
    D: Buf,
    StreamBody<ByteStreamBody<S, D>>: Body,
{
    /// Alias of [`from_bytes_stream`](Self::from_bytes_stream).
    #[deprecated(
        since = "0.2.4",
        note = "It's a typo, use `from_bytes_stream` instead. This method will be removed in 0.3.0"
    )]
    pub fn from_byte_stream(stream: S) -> Self {
        Self::from_bytes_stream(stream)
    }

    /// Create a new [`SseStream`] from a stream of [`Bytes`](bytes::Bytes).
    ///
    /// This is useful when you interact with clients don't provide response body directly like reqwest.
    pub fn from_bytes_stream(stream: S) -> Self {
        let stream = stream.map_ok(http_body::Frame::data as fn(D) -> Frame<D>);
        let body = StreamBody::new(stream);
        Self {
            body: BodyDataStream::new(body),
            parser: ParserState::default(),
        }
    }
}

impl<B: Body> SseStream<B> {
    /// Create a new [`SseStream`] from a [`Body`].
    pub fn new(body: B) -> Self {
        Self {
            body: BodyDataStream::new(body),
            parser: ParserState::default(),
        }
    }
}

pub enum Error {
    Body(Box<dyn std::error::Error + Send + Sync>),
    InvalidLine,
    DuplicatedEventLine,
    DuplicatedIdLine,
    DuplicatedRetry,
    Utf8Parse(Utf8Error),
    IntParse(ParseIntError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Body(e) => write!(f, "body error: {}", e),
            Error::InvalidLine => write!(f, "invalid line"),
            Error::DuplicatedEventLine => write!(f, "duplicated event line"),
            Error::DuplicatedIdLine => write!(f, "duplicated id line"),
            Error::DuplicatedRetry => write!(f, "duplicated retry line"),
            Error::Utf8Parse(e) => write!(f, "utf8 parse error: {}", e),
            Error::IntParse(e) => write!(f, "int parse error: {}", e),
        }
    }
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Body(e) => write!(f, "Body({:?})", e),
            Error::InvalidLine => write!(f, "InvalidLine"),
            Error::DuplicatedEventLine => write!(f, "DuplicatedEventLine"),
            Error::DuplicatedIdLine => write!(f, "DuplicatedIdLine"),
            Error::DuplicatedRetry => write!(f, "DuplicatedRetry"),
            Error::Utf8Parse(e) => write!(f, "Utf8Parse({:?})", e),
            Error::IntParse(e) => write!(f, "IntParse({:?})", e),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match self {
            Error::Body(_) => "body error",
            Error::InvalidLine => "invalid line",
            Error::DuplicatedEventLine => "duplicated event line",
            Error::DuplicatedIdLine => "duplicated id line",
            Error::DuplicatedRetry => "duplicated retry line",
            Error::Utf8Parse(_) => "utf8 parse error",
            Error::IntParse(_) => "int parse error",
        }
    }

    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Body(e) => Some(e.as_ref()),
            Error::Utf8Parse(e) => Some(e),
            Error::IntParse(e) => Some(e),
            _ => None,
        }
    }
}

impl ParserState {
    fn parse_line(&mut self, mut line: &[u8]) -> Result<(), Error> {
        if self.first_line {
            self.first_line = false;
            line = line.strip_prefix(BOM_HEADER).unwrap_or(line);
        }

        if line.is_empty() {
            if let Some(sse) = self.current.take() {
                self.parsed.push_back(sse);
            }
            return Ok(());
        }

        let Some(colon_index) = line.iter().position(|byte| *byte == b':') else {
            #[cfg(feature = "tracing")]
            tracing::warn!(?line, "invalid line, missing `:`");
            return Err(Error::InvalidLine);
        };
        let field_name = &line[..colon_index];
        let field_value = &line[colon_index + 1..];
        let field_value = field_value.strip_prefix(b" ").unwrap_or(field_value);

        match field_name {
            b"data" => {
                let data_line = std::str::from_utf8(field_value).map_err(Error::Utf8Parse)?;
                let event = self.current.get_or_insert_default();
                if let Some(data) = event.data.as_mut() {
                    data.push('\n');
                    data.push_str(data_line);
                } else {
                    event.data = Some(data_line.to_owned());
                }
            }
            b"event" => {
                let event_value = std::str::from_utf8(field_value).map_err(Error::Utf8Parse)?;
                let event = self.current.get_or_insert_default();
                if event.event.is_some() {
                    return Err(Error::DuplicatedEventLine);
                }
                event.event = Some(event_value.to_owned());
            }
            b"id" => {
                // Per spec: if the id field value contains U+0000 NULL,
                // the entire field MUST be ignored.
                if field_value.contains(&0_u8) {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(?line, "id field contains NULL byte, ignoring per spec");
                    return Ok(());
                }
                let id_value = std::str::from_utf8(field_value).map_err(Error::Utf8Parse)?;
                let event = self.current.get_or_insert_default();
                if event.id.is_some() {
                    return Err(Error::DuplicatedIdLine);
                }
                event.id = Some(id_value.to_owned());
            }
            b"retry" => {
                let retry_value = std::str::from_utf8(field_value)
                    .map_err(Error::Utf8Parse)?
                    .trim_ascii()
                    .parse::<u64>()
                    .map_err(Error::IntParse)?;
                let event = self.current.get_or_insert_default();
                if event.retry.is_some() {
                    return Err(Error::DuplicatedRetry);
                }
                event.retry = Some(retry_value);
            }
            b"" => {
                #[cfg(feature = "tracing")]
                {
                    if tracing::enabled!(tracing::Level::DEBUG) {
                        let comment = std::str::from_utf8(field_value).map_err(Error::Utf8Parse)?;
                        tracing::debug!(?comment, "sse comment line");
                    }
                }
            }
            _ => {
                #[cfg(feature = "tracing")]
                tracing::warn!(line = ?field_name, "invalid line: unknown field");
                return Err(Error::InvalidLine);
            }
        }

        Ok(())
    }

    fn parse_complete_line(&mut self, line: &[u8]) -> Result<(), Error> {
        // Fast path to avoid copy overhead if we don't have anything buffered.
        if self.unfinished_line.is_empty() {
            self.parse_line(line)
        } else {
            let mut complete_line = std::mem::take(&mut self.unfinished_line);
            complete_line.extend_from_slice(line);
            let result = self.parse_line(&complete_line);
            // Reuse the unfinished line buffer.
            complete_line.clear();
            self.unfinished_line = complete_line;
            result
        }
    }

    fn parse_chunk(&mut self, mut bytes: &[u8]) -> Result<(), Error> {
        if self.skip_leading_lf {
            self.skip_leading_lf = false;
            if bytes[0] == b'\n' {
                bytes = &bytes[1..];
            }
        }

        while !bytes.is_empty() {
            let Some(line_end) = bytes.iter().position(|byte| matches!(*byte, b'\n' | b'\r'))
            else {
                self.unfinished_line.extend_from_slice(bytes);
                return Ok(());
            };

            self.parse_complete_line(&bytes[..line_end])?;

            let delimiter = bytes[line_end];
            bytes = &bytes[line_end + 1..];
            if delimiter == b'\r' {
                if bytes.first() == Some(&b'\n') {
                    bytes = &bytes[1..];
                } else if bytes.is_empty() {
                    self.skip_leading_lf = true;
                }
            }
        }

        Ok(())
    }
}

impl<B: Body> Stream for SseStream<B>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<Sse, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        if let Some(sse) = this.parser.parsed.pop_front() {
            return Poll::Ready(Some(Ok(sse)));
        }
        loop {
            match ready!(this.body.as_mut().poll_next(cx)) {
                Some(Err(error)) => return Poll::Ready(Some(Err(Error::Body(Box::new(error))))),
                None => return Poll::Ready(None),
                Some(Ok(mut data)) => {
                    while data.has_remaining() {
                        let bytes = data.chunk();
                        debug_assert!(
                            !bytes.is_empty(),
                            "Buf::chunk returned an empty slice with bytes remaining"
                        );
                        let chunk_size = bytes.len();
                        if let Err(error) = this.parser.parse_chunk(bytes) {
                            return Poll::Ready(Some(Err(error)));
                        }
                        data.advance(chunk_size);
                    }

                    if let Some(sse) = this.parser.parsed.pop_front() {
                        return Poll::Ready(Some(Ok(sse)));
                    }
                }
            }
        }
    }
}
