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
    /// Bytes of an incomplete line that must be parsed whole: anything not
    /// yet known to be a `data` field value.
    pending_line: Vec<u8>,
    /// Payload bytes of the `data` fields of the event in `current`, with a
    /// `'\n'` separator after each line that is dropped at dispatch. Validated
    /// line by line; may briefly hold unvalidated bytes of a line in progress.
    data_buf: Vec<u8>,
    /// A `data` line currently being streamed into `data_buf` across chunks.
    pending_data_line: Option<PendingDataLine>,
    skip_leading_lf: bool,
    first_line: bool,
}

/// A `data` line being streamed into `data_buf` across chunks.
#[derive(Clone, Copy)]
struct PendingDataLine {
    /// Start index of the line's value bytes in `data_buf`.
    line_start: usize,
    /// Whether the optional single space after `data:` has been consumed yet.
    strip_leading_space: bool,
}

impl Default for ParserState {
    fn default() -> Self {
        Self {
            parsed: VecDeque::new(),
            current: None,
            pending_line: Vec::new(),
            data_buf: Vec::new(),
            pending_data_line: None,
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
            if let Some(mut sse) = self.current.take() {
                if !self.data_buf.is_empty() {
                    // Drop the separator of the last data line.
                    self.data_buf.pop();
                    let data_bytes = std::mem::take(&mut self.data_buf);
                    // SAFETY: `data_buf` only ever contains `data` field values
                    // joined by the ASCII separator b'\n', and every byte of
                    // those values passed `str::from_utf8` — either in the
                    // `data` branch of `parse_line` before being appended, or
                    // in `finish_data_line` right after, where invalid bytes
                    // are truncated away.
                    let data = unsafe { String::from_utf8_unchecked(data_bytes) };
                    // Keep the allocation for the next event.
                    self.data_buf = Vec::with_capacity(data.capacity());
                    sse.data = Some(data);
                }
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
                // Accumulate validated values; the `String` is built at dispatch.
                let data_line = std::str::from_utf8(field_value).map_err(Error::Utf8Parse)?;
                self.current.get_or_insert_default();
                self.data_buf.extend_from_slice(data_line.as_bytes());
                self.data_buf.push(b'\n');
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
        if self.pending_line.is_empty() {
            self.parse_line(line)
        } else {
            let mut complete_line = std::mem::take(&mut self.pending_line);
            complete_line.extend_from_slice(line);
            let result = self.parse_line(&complete_line);
            // Keep the allocation for the next fragmented line.
            complete_line.clear();
            self.pending_line = complete_line;
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

        // A pending data line resumes here. It can only be pending across
        // chunks: the branch that starts one below always returns.
        if let Some(mut pending) = self.pending_data_line {
            if pending.strip_leading_space {
                // Consume the optional single space if the chunk split fell
                // between `data:` and its value.
                pending.strip_leading_space = false;
                self.pending_data_line = Some(pending);
                if bytes.first() == Some(&b' ') {
                    bytes = &bytes[1..];
                }
            }
            match find_line_end(bytes) {
                None => {
                    self.data_buf.extend_from_slice(bytes);
                    return Ok(());
                }
                Some(line_end) => {
                    self.data_buf.extend_from_slice(&bytes[..line_end]);
                    self.finish_data_line(pending.line_start)?;
                    bytes = self.advance_past_delimiter(bytes, line_end);
                }
            }
        }

        while !bytes.is_empty() {
            let Some(line_end) = find_line_end(bytes) else {
                // Incomplete line. A `data:` prefix means the value can stream
                // into `data_buf`; anything else is buffered until the line
                // completes.
                if self.pending_line.is_empty() && bytes.starts_with(b"data:") {
                    let line_start = self.data_buf.len();
                    let value = bytes[5..].strip_prefix(b" ").unwrap_or(&bytes[5..]);
                    self.data_buf.extend_from_slice(value);
                    self.current.get_or_insert_default();
                    self.pending_data_line = Some(PendingDataLine {
                        line_start,
                        strip_leading_space: bytes.len() == 5, // ends right after `:`
                    });
                } else {
                    self.pending_line.extend_from_slice(bytes);
                }
                return Ok(());
            };

            self.parse_complete_line(&bytes[..line_end])?;
            bytes = self.advance_past_delimiter(bytes, line_end);
        }

        Ok(())
    }

    /// Consume the line ending at `bytes[line_end]` and return the rest of
    /// `bytes`. A `\r` at the very end of `bytes` is remembered so a leading
    /// `\n` in the next chunk can be skipped.
    #[inline]
    fn advance_past_delimiter<'a>(&mut self, bytes: &'a [u8], line_end: usize) -> &'a [u8] {
        let rest = &bytes[line_end + 1..];
        if bytes[line_end] == b'\r' {
            if rest.first() == Some(&b'\n') {
                &rest[1..]
            } else {
                if rest.is_empty() {
                    self.skip_leading_lf = true;
                }
                rest
            }
        } else {
            rest
        }
    }

    /// Validate the data line at `data_buf[line_start..]` in place and
    /// terminate it with the `'\n'` separator. Invalid UTF-8 truncates the
    /// partial line and is reported as a parse error.
    #[inline]
    fn finish_data_line(&mut self, line_start: usize) -> Result<(), Error> {
        self.pending_data_line = None;
        if let Err(err) = std::str::from_utf8(&self.data_buf[line_start..]) {
            self.data_buf.truncate(line_start);
            return Err(Error::Utf8Parse(err));
        }
        self.data_buf.push(b'\n');
        Ok(())
    }
}

/// Find the index of the first `\n` or `\r` in `bytes`.
///
/// The first few bytes are scanned scalar, the rest with
/// [`memchr2`](memchr::memchr2): SIMD dispatch does not pay off on short lines.
#[cfg(feature = "memchr")]
#[inline]
fn find_line_end(bytes: &[u8]) -> Option<usize> {
    /// How many leading bytes are scanned scalar before delegating to `memchr2`.
    const SCALAR_HEAD: usize = 16;

    let head_len = bytes.len().min(SCALAR_HEAD);
    let head_hit = bytes[..head_len]
        .iter()
        .position(|byte| matches!(*byte, b'\n' | b'\r'));
    if head_len == bytes.len() || head_hit.is_some() {
        return head_hit;
    }
    memchr::memchr2(b'\n', b'\r', &bytes[head_len..]).map(|i| i + head_len)
}

/// Find the index of the first `\n` or `\r` in `bytes`.
#[cfg(not(feature = "memchr"))]
#[inline]
fn find_line_end(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|byte| matches!(*byte, b'\n' | b'\r'))
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
