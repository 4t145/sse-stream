use std::{
    num::ParseIntError,
    str::Utf8Error,
    task::{ready, Context, Poll},
};

use crate::Sse;
use bytes::Buf;
use futures_util::{stream::MapOk, Stream, TryStream, TryStreamExt};
use http_body::{Body, Frame};
use http_body_util::{BodyDataStream, StreamBody};

const BOM_HEADER: &[u8] = b"\xEF\xBB\xBF";
const DATA_FIELD_PREFIX: &[u8] = b"data:";

struct ParserState {
    has_fields: bool,
    event: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
    /// Bytes of an incomplete line that must be parsed whole: anything not
    /// yet known to be a `data` field value.
    pending_line: Vec<u8>,
    /// Payload bytes of the `data` fields of the current event, with a
    /// `'\n'` separator after each line that is dropped at dispatch. Validated
    /// line by line; may briefly hold unvalidated bytes of a line in progress.
    data_buf: Vec<u8>,
    /// A `data` line currently being streamed into `data_buf` across chunks.
    pending_data_line: Option<PendingDataLine>,
    /// A comment split across chunks whose contents need not be retained.
    discarding_comment: bool,
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
            has_fields: false,
            event: None,
            id: None,
            retry: None,
            pending_line: Vec::new(),
            data_buf: Vec::new(),
            pending_data_line: None,
            discarding_comment: false,
            skip_leading_lf: false,
            first_line: true,
        }
    }
}

pin_project_lite::pin_project! {
    /// An SSE decoder over an [`http_body::Body`].
    pub struct SseStream<B: Body> {
        #[pin]
        body: BodyDataStream<B>,
        data: Option<B::Data>,
        parser: ParserState,
    }
}

pin_project_lite::pin_project! {
    /// An SSE decoder over a stream of byte buffers.
    ///
    /// Unlike [`SseStream`], this type consumes the byte stream directly and
    /// does not convert every buffer into an HTTP [`Frame`] and back.
    pub struct SseByteStream<S: TryStream> {
        #[pin]
        stream: S,
        data: Option<S::Ok>,
        parser: ParserState,
    }
}

impl<S: TryStream> SseByteStream<S> {
    /// Create an [`SseByteStream`] from a stream of byte buffers.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            data: None,
            parser: ParserState::default(),
        }
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
    /// New code should prefer [`SseByteStream::new`], which avoids adapting the
    /// byte stream through [`StreamBody`] and [`BodyDataStream`].
    pub fn from_bytes_stream(stream: S) -> Self {
        let stream = stream.map_ok(http_body::Frame::data as fn(D) -> Frame<D>);
        let body = StreamBody::new(stream);
        Self {
            body: BodyDataStream::new(body),
            data: None,
            parser: ParserState::default(),
        }
    }
}

impl<B: Body> SseStream<B> {
    /// Create a new [`SseStream`] from a [`Body`].
    pub fn new(body: B) -> Self {
        Self {
            body: BodyDataStream::new(body),
            data: None,
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
    fn dispatch(&mut self) -> Option<Sse> {
        if !self.has_fields {
            return None;
        }
        self.has_fields = false;

        let data = if self.data_buf.is_empty() {
            None
        } else {
            // Drop the separator of the last data line.
            self.data_buf.pop();
            let data_bytes = if self.data_buf.capacity() > 4096
                && self.data_buf.len() < self.data_buf.capacity() / 4
            {
                // Do not make small outputs inherit a previous large event's
                // capacity. Retain the staging buffer for future fragmented
                // events, and copy only the small current payload.
                let data = self.data_buf.clone();
                self.data_buf.clear();
                data
            } else {
                let data = std::mem::take(&mut self.data_buf);
                // Transfer large outputs without another payload copy, while
                // reserving space for the next similarly sized event.
                self.data_buf = Vec::with_capacity(data.capacity());
                data
            };
            // SAFETY: `data_buf` only ever contains `data` field values joined
            // by the ASCII separator b'\n', and every byte of those values
            // passed UTF-8 validation before dispatch.
            let data = unsafe { String::from_utf8_unchecked(data_bytes) };
            Some(data)
        };

        Some(Sse {
            event: self.event.take(),
            data,
            id: self.id.take(),
            retry: self.retry.take(),
        })
    }

    #[inline]
    fn push_data_line(&mut self, field_value: &[u8]) -> Result<(), Error> {
        let data_line = validate_utf8(field_value).map_err(Error::Utf8Parse)?;
        self.has_fields = true;
        self.data_buf.extend_from_slice(data_line.as_bytes());
        self.data_buf.push(b'\n');
        Ok(())
    }

    fn parse_line(&mut self, mut line: &[u8]) -> Result<Option<Sse>, Error> {
        if self.first_line {
            self.first_line = false;
            line = line.strip_prefix(BOM_HEADER).unwrap_or(line);
        }

        if line.is_empty() {
            return Ok(self.dispatch());
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
                self.push_data_line(field_value)?;
            }
            b"event" => {
                let event_value = validate_utf8(field_value).map_err(Error::Utf8Parse)?;
                if self.event.is_some() {
                    return Err(Error::DuplicatedEventLine);
                }
                self.has_fields = true;
                self.event = Some(event_value.to_owned());
            }
            b"id" => {
                // Per spec: if the id field value contains U+0000 NULL,
                // the entire field MUST be ignored.
                if field_value.contains(&0_u8) {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(?line, "id field contains NULL byte, ignoring per spec");
                    return Ok(None);
                }
                let id_value = validate_utf8(field_value).map_err(Error::Utf8Parse)?;
                if self.id.is_some() {
                    return Err(Error::DuplicatedIdLine);
                }
                self.has_fields = true;
                self.id = Some(id_value.to_owned());
            }
            b"retry" => {
                let retry_value = validate_utf8(field_value)
                    .map_err(Error::Utf8Parse)?
                    .trim_ascii()
                    .parse::<u64>()
                    .map_err(Error::IntParse)?;
                if self.retry.is_some() {
                    return Err(Error::DuplicatedRetry);
                }
                self.has_fields = true;
                self.retry = Some(retry_value);
            }
            b"" => {
                #[cfg(feature = "tracing")]
                {
                    if tracing::enabled!(tracing::Level::DEBUG) {
                        let comment = validate_utf8(field_value).map_err(Error::Utf8Parse)?;
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

        Ok(None)
    }

    fn parse_complete_line(&mut self, line: &[u8]) -> Result<Option<Sse>, Error> {
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

    fn parse_chunk(&mut self, mut bytes: &[u8]) -> Result<(usize, Option<Sse>), Error> {
        let original_len = bytes.len();

        if self.skip_leading_lf {
            self.skip_leading_lf = false;
            if bytes[0] == b'\n' {
                bytes = &bytes[1..];
            }
        }

        // Comments have no semantic payload. Unless debug tracing needs their
        // contents, consume a fragmented comment without copying it through
        // `pending_line`.
        if self.discarding_comment {
            let Some(line_end) = find_line_end(bytes) else {
                return Ok((original_len, None));
            };
            self.discarding_comment = false;
            bytes = self.advance_past_delimiter(bytes, line_end);
            if !self.has_fields {
                bytes = match bytes.first() {
                    Some(b'\n') => &bytes[1..],
                    Some(b'\r') => self.advance_past_delimiter(bytes, 0),
                    _ => bytes,
                };
            }
        }

        // If the previous chunk ended inside `data:`, finish recognizing the
        // field prefix before falling back to buffering the complete line.
        // This keeps a split prefix such as `data` + `: value` on the streaming
        // data path instead of copying the value through `pending_line` first.
        if !self.pending_line.is_empty()
            && self.pending_line.len() < DATA_FIELD_PREFIX.len()
            && DATA_FIELD_PREFIX.starts_with(&self.pending_line)
        {
            let missing_prefix = &DATA_FIELD_PREFIX[self.pending_line.len()..];
            if bytes.len() < missing_prefix.len() {
                if missing_prefix.starts_with(bytes) {
                    self.pending_line.extend_from_slice(bytes);
                    return Ok((original_len, None));
                }
            } else if bytes.starts_with(missing_prefix) {
                bytes = &bytes[missing_prefix.len()..];
                self.pending_line.clear();
                self.first_line = false;
                self.has_fields = true;
                self.pending_data_line = Some(PendingDataLine {
                    line_start: self.data_buf.len(),
                    strip_leading_space: true,
                });
            }
        }

        // A pending data line resumes here. It can only be pending across
        // chunks: the branch that starts one below always returns.
        if let Some(mut pending) = self.pending_data_line {
            if pending.strip_leading_space && !bytes.is_empty() {
                // Consume the optional single space if the chunk split fell
                // between `data:` and its value. Completing a split prefix can
                // leave no value bytes yet; keep the flag for the next chunk.
                pending.strip_leading_space = false;
                self.pending_data_line = Some(pending);
                if bytes.first() == Some(&b' ') {
                    bytes = &bytes[1..];
                }
            }
            match find_line_end(bytes) {
                None => {
                    self.data_buf.extend_from_slice(bytes);
                    return Ok((original_len, None));
                }
                Some(line_end) => {
                    self.data_buf.extend_from_slice(&bytes[..line_end]);
                    self.finish_data_line(pending.line_start)?;
                    bytes = self.advance_past_delimiter(bytes, line_end);
                }
            }
        }

        while !bytes.is_empty() {
            if self.pending_line.is_empty() && bytes[0] == b':' {
                let Some(line_end) = find_line_end(bytes) else {
                    if comment_tracing_enabled() {
                        self.pending_line.extend_from_slice(bytes);
                    } else {
                        self.first_line = false;
                        self.discarding_comment = true;
                    }
                    return Ok((original_len, None));
                };
                // Comments dominate idle streams. Parse tracing data only when
                // it can actually be observed, then skip the common blank line
                // that follows a standalone keepalive.
                self.first_line = false;
                #[cfg(feature = "tracing")]
                if tracing::enabled!(tracing::Level::DEBUG) {
                    let value = &bytes[1..line_end];
                    let value = value.strip_prefix(b" ").unwrap_or(value);
                    let comment = validate_utf8(value).map_err(Error::Utf8Parse)?;
                    tracing::debug!(?comment, "sse comment line");
                }

                bytes = self.advance_past_delimiter(bytes, line_end);
                if !self.has_fields {
                    bytes = match bytes.first() {
                        Some(b'\n') => &bytes[1..],
                        Some(b'\r') => self.advance_past_delimiter(bytes, 0),
                        _ => bytes,
                    };
                }
                continue;
            }
            let data_value = if self.pending_line.is_empty() {
                bytes.strip_prefix(DATA_FIELD_PREFIX)
            } else {
                None
            };
            let line_end = match data_value {
                Some(value) => find_data_line_end(value).map(|end| end + DATA_FIELD_PREFIX.len()),
                None => find_line_end(bytes),
            };
            let Some(line_end) = line_end else {
                // Incomplete line. A `data:` prefix means the value can stream
                // into `data_buf`; anything else is buffered until the line
                // completes.
                if let Some(value) = data_value {
                    self.first_line = false;
                    let line_start = self.data_buf.len();
                    let value = value.strip_prefix(b" ").unwrap_or(value);
                    self.data_buf.extend_from_slice(value);
                    self.has_fields = true;
                    self.pending_data_line = Some(PendingDataLine {
                        line_start,
                        strip_leading_space: bytes.len() == DATA_FIELD_PREFIX.len(),
                    });
                } else {
                    self.pending_line.extend_from_slice(bytes);
                }
                return Ok((original_len, None));
            };

            let event = if self.pending_line.is_empty() && line_end == 0 {
                // Blank lines are event boundaries. Keep this common path out
                // of the generic field parser.
                self.first_line = false;
                self.dispatch()
            } else if let Some(value) = data_value {
                // `data` dominates active streams. Its fixed field name means
                // there is no need to scan the line for a colon and dispatch
                // through the generic field match.
                self.first_line = false;
                let value = &value[..line_end - DATA_FIELD_PREFIX.len()];
                let value = value.strip_prefix(b" ").unwrap_or(value);
                self.push_data_line(value)?;
                // A following LF blank line completes the event in this iteration.
                if bytes[line_end..].starts_with(b"\n\n") {
                    return Ok((original_len - bytes.len() + line_end + 2, self.dispatch()));
                }
                None
            } else {
                self.parse_complete_line(&bytes[..line_end])?
            };
            bytes = self.advance_past_delimiter(bytes, line_end);
            if let Some(event) = event {
                return Ok((original_len - bytes.len(), Some(event)));
            }
        }

        Ok((original_len, None))
    }

    fn parse_buf(&mut self, data: &mut impl Buf) -> Result<Option<Sse>, Error> {
        while data.has_remaining() {
            let bytes = data.chunk();
            debug_assert!(
                !bytes.is_empty(),
                "Buf::chunk returned an empty slice with bytes remaining"
            );
            let (consumed, event) = self.parse_chunk(bytes)?;
            debug_assert!(consumed > 0 && consumed <= bytes.len());
            data.advance(consumed);
            if event.is_some() {
                return Ok(event);
            }
        }
        Ok(None)
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
        if let Err(err) = validate_utf8(&self.data_buf[line_start..]) {
            self.data_buf.truncate(line_start);
            return Err(Error::Utf8Parse(err));
        }
        self.data_buf.push(b'\n');
        Ok(())
    }
}

#[inline]
fn validate_utf8(value: &[u8]) -> Result<&str, Utf8Error> {
    #[cfg(feature = "simdutf8")]
    if value.len() >= 256 {
        // Short fields avoid SIMD dispatch. On failure, retain the standard
        // library's error type, valid_up_to(), and error_len().
        if let Ok(text) = simdutf8::basic::from_utf8(value) {
            return Ok(text);
        }
    }
    std::str::from_utf8(value)
}

#[inline]
fn comment_tracing_enabled() -> bool {
    #[cfg(feature = "tracing")]
    {
        tracing::enabled!(tracing::Level::DEBUG)
    }
    #[cfg(not(feature = "tracing"))]
    {
        false
    }
}

#[inline]
fn find_data_line_end(bytes: &[u8]) -> Option<usize> {
    #[cfg(feature = "memchr")]
    {
        const SCALAR_MAX_LEN: usize = 16;
        if bytes.len() > SCALAR_MAX_LEN {
            return memchr::memchr2(b'\n', b'\r', bytes);
        }
    }
    bytes.iter().position(|byte| matches!(*byte, b'\n' | b'\r'))
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
        loop {
            if let Some(data) = this.data.as_mut() {
                match this.parser.parse_buf(data) {
                    Ok(Some(sse)) => {
                        if !data.has_remaining() {
                            *this.data = None;
                        }
                        return Poll::Ready(Some(Ok(sse)));
                    }
                    Ok(None) => *this.data = None,
                    Err(error) => {
                        *this.data = None;
                        return Poll::Ready(Some(Err(error)));
                    }
                }
            }

            match ready!(this.body.as_mut().poll_next(cx)) {
                Some(Err(error)) => return Poll::Ready(Some(Err(Error::Body(Box::new(error))))),
                None => return Poll::Ready(None),
                Some(Ok(mut data)) => match this.parser.parse_buf(&mut data) {
                    Ok(Some(sse)) => {
                        if data.has_remaining() {
                            *this.data = Some(data);
                        }
                        return Poll::Ready(Some(Ok(sse)));
                    }
                    Ok(None) => {}
                    Err(error) => return Poll::Ready(Some(Err(error))),
                },
            }
        }
    }
}

impl<S> Stream for SseByteStream<S>
where
    S: TryStream,
    S::Ok: Buf,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<Sse, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        loop {
            if let Some(data) = this.data.as_mut() {
                match this.parser.parse_buf(data) {
                    Ok(Some(sse)) => {
                        if !data.has_remaining() {
                            *this.data = None;
                        }
                        return Poll::Ready(Some(Ok(sse)));
                    }
                    Ok(None) => *this.data = None,
                    Err(error) => {
                        *this.data = None;
                        return Poll::Ready(Some(Err(error)));
                    }
                }
            }

            match ready!(this.stream.as_mut().try_poll_next(cx)) {
                Some(Err(error)) => return Poll::Ready(Some(Err(Error::Body(Box::new(error))))),
                None => return Poll::Ready(None),
                Some(Ok(mut data)) => match this.parser.parse_buf(&mut data) {
                    Ok(Some(sse)) => {
                        if data.has_remaining() {
                            *this.data = Some(data);
                        }
                        return Poll::Ready(Some(Ok(sse)));
                    }
                    Ok(None) => {}
                    Err(error) => return Poll::Ready(Some(Err(error))),
                },
            }
        }
    }
}
