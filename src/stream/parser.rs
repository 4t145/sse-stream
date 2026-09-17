use bytes::Buf;

use crate::{Error, Sse};

mod data;
mod scan;

use data::{DataBuffer, MAX_RETAINED_CAPACITY};
use scan::{find_data_line_end, find_line_end, validate_utf8};

const BOM_HEADER: &[u8] = b"\xEF\xBB\xBF";
const DATA_FIELD_PREFIX: &[u8] = b"data:";

#[derive(Default)]
struct EventBuilder {
    event: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
    data: DataBuffer,
    has_fields: bool,
}

impl EventBuilder {
    #[inline]
    fn dispatch(&mut self) -> Option<Sse> {
        if !self.has_fields {
            return None;
        }
        Some(self.finish())
    }

    #[inline]
    fn finish(&mut self) -> Sse {
        self.has_fields = false;
        Sse {
            event: self.event.take(),
            id: self.id.take(),
            retry: self.retry.take(),
            data: self.data.take(),
        }
    }

    /// # Errors
    /// Returns an error for an invalid UTF-8 data value.
    #[inline]
    fn push_data_line(&mut self, value: &[u8]) -> Result<(), Error> {
        self.data.push_line(value)?;
        self.has_fields = true;
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
enum LineMode {
    // To parse a new line
    #[default]
    Start,
    Buffered,
    Data {
        needs_leading_space_check: bool,
    },
    Comment,
}

pub(super) struct Parser {
    event: EventBuilder,
    mode: LineMode,
    pending_line: Vec<u8>,
    skip_leading_lf: bool,
    first_line: bool,
}

impl Default for Parser {
    fn default() -> Self {
        Self {
            event: EventBuilder::default(),
            mode: LineMode::Start,
            pending_line: Vec::new(),
            skip_leading_lf: false,
            first_line: true,
        }
    }
}

struct ParsedEvent {
    consumed: usize,
    event: Sse,
}

impl Parser {
    /// # Errors
    /// Returns an error for invalid UTF-8 in a recognized field value or an
    /// unknown field when `strict-fields` is enabled.
    pub(super) fn parse_buf(&mut self, data: &mut impl Buf) -> Result<Option<Sse>, Error> {
        while data.has_remaining() {
            let bytes = data.chunk();
            debug_assert!(!bytes.is_empty(), "Buf::chunk must make progress");
            let chunk_len = bytes.len();
            if let Some(parsed) = self.parse_chunk(bytes)? {
                debug_assert!(parsed.consumed > 0 && parsed.consumed <= chunk_len);
                data.advance(parsed.consumed);
                return Ok(Some(parsed.event));
            }
            data.advance(chunk_len);
        }
        Ok(None)
    }

    /// Without an event, the whole chunk is consumed.
    ///
    /// # Errors
    /// Returns an error for invalid UTF-8 in a recognized field value or an
    /// unknown field when `strict-fields` is enabled.
    fn parse_chunk(&mut self, mut bytes: &[u8]) -> Result<Option<ParsedEvent>, Error> {
        let original_len = bytes.len();
        if self.skip_leading_lf {
            // The previous chunk ended with CR; skip an optional LF completing CRLF.
            self.skip_leading_lf = false;
            bytes = bytes.strip_prefix(b"\n").unwrap_or(bytes);
        }
        if !matches!(self.mode, LineMode::Start) {
            // Continue a line split across chunks.
            bytes = self.resume_line(bytes)?;
        }

        while !bytes.is_empty() {
            if matches!(self.mode, LineMode::Start) {
                // Only new lines can use the comment, data, and empty-line shortcuts.
                if bytes[0] == b':' {
                    // A colon at the start of a line introduces a comment.
                    bytes = self.consume_comment_line(bytes);
                    continue;
                }
                if let Some(bytes_after_data_prefix) = bytes.strip_prefix(DATA_FIELD_PREFIX) {
                    // The complete data prefix lets us append the value directly.
                    self.first_line = false;
                    let Some(value_end) = find_data_line_end(bytes_after_data_prefix) else {
                        // The data line continues in the next chunk.
                        let fragment = bytes_after_data_prefix;
                        self.event
                            .data
                            .push_fragment(fragment.strip_prefix(b" ").unwrap_or(fragment));
                        self.mode = LineMode::Data {
                            needs_leading_space_check: fragment.is_empty(),
                        };
                        return Ok(None);
                    };

                    let value = &bytes_after_data_prefix[..value_end];
                    self.event
                        .push_data_line(value.strip_prefix(b" ").unwrap_or(value))?;
                    let line_end = DATA_FIELD_PREFIX.len() + value_end;
                    if bytes[line_end..].starts_with(b"\n\n") {
                        // LF followed by an empty LF line completes the event immediately.
                        return Ok(Some(ParsedEvent {
                            consumed: original_len - bytes.len() + line_end + 2,
                            event: self.event.finish(),
                        }));
                    }
                    bytes = self.advance_past_delimiter(bytes, line_end);
                    continue;
                }
                if matches!(bytes[0], b'\n' | b'\r') {
                    // An empty new line completes the event without field parsing.
                    self.first_line = false;
                    let event = self.event.dispatch();
                    bytes = self.advance_past_delimiter(bytes, 0);
                    if let Some(event) = event {
                        // Return the event after consuming its empty-line delimiter.
                        return Ok(Some(ParsedEvent {
                            consumed: original_len - bytes.len(),
                            event,
                        }));
                    }
                    continue;
                }
            }

            // Other fields and buffered continuations share the general path.
            let Some(line_end) = find_line_end(bytes) else {
                // Retain the unparsed bytes until the line is complete.
                self.buffer_line(bytes);
                return Ok(None);
            };
            let event = self.finish_buffered_line(&bytes[..line_end])?;
            bytes = self.advance_past_delimiter(bytes, line_end);
            if let Some(event) = event {
                // Return the completed event, leaving the remaining bytes for the next call.
                return Ok(Some(ParsedEvent {
                    consumed: original_len - bytes.len(),
                    event,
                }));
            }
        }
        Ok(None)
    }

    /// Consume a new comment line and an optional idle boundary, returning unconsumed input.
    #[inline]
    fn consume_comment_line<'a>(&mut self, bytes: &'a [u8]) -> &'a [u8] {
        self.first_line = false;
        let Some(line_end) = find_line_end(bytes) else {
            // The comment continues in the next chunk.
            if comment_tracing_enabled() {
                // Retain the text so the complete comment can be traced.
                self.buffer_line(bytes);
            } else {
                // Discard the text and keep skipping until the line ends.
                self.mode = LineMode::Comment;
            }
            return &[];
        };
        trace_comment(&bytes[1..line_end]);
        let bytes = self.advance_past_delimiter(bytes, line_end);
        self.skip_idle_boundary(bytes)
    }

    /// Resume a fragmented line, returning the unconsumed input.
    ///
    /// # Errors
    /// Returns an error when a completed data line is not valid UTF-8.
    #[inline]
    fn resume_line<'a>(&mut self, mut bytes: &'a [u8]) -> Result<&'a [u8], Error> {
        if matches!(self.mode, LineMode::Comment) {
            let Some(line_end) = find_line_end(bytes) else {
                return Ok(&[]);
            };
            self.mode = LineMode::Start;
            bytes = self.advance_past_delimiter(bytes, line_end);
            return Ok(self.skip_idle_boundary(bytes));
        }
        if matches!(self.mode, LineMode::Buffered) {
            bytes = self.resume_data_prefix(bytes);
        }
        if let LineMode::Data {
            mut needs_leading_space_check,
        } = self.mode
        {
            if needs_leading_space_check && !bytes.is_empty() {
                bytes = bytes.strip_prefix(b" ").unwrap_or(bytes);
                needs_leading_space_check = false;
                self.mode = LineMode::Data {
                    needs_leading_space_check,
                };
            }
            let Some(line_end) = find_line_end(bytes) else {
                self.event.data.push_fragment(bytes);
                return Ok(&[]);
            };
            self.event.data.push_fragment(&bytes[..line_end]);
            self.event.data.finish_line()?;
            self.event.has_fields = true;
            self.mode = LineMode::Start;
            bytes = self.advance_past_delimiter(bytes, line_end);
        }
        Ok(bytes)
    }

    #[inline]
    fn resume_data_prefix<'a>(&mut self, bytes: &'a [u8]) -> &'a [u8] {
        if self.pending_line.len() >= DATA_FIELD_PREFIX.len()
            || !DATA_FIELD_PREFIX.starts_with(&self.pending_line)
        {
            return bytes;
        }
        let missing = &DATA_FIELD_PREFIX[self.pending_line.len()..];
        if bytes.len() < missing.len() {
            if missing.starts_with(bytes) {
                self.pending_line.extend_from_slice(bytes);
                return &[];
            }
        } else if let Some(rest) = bytes.strip_prefix(missing) {
            self.pending_line.clear();
            self.first_line = false;
            self.mode = LineMode::Data {
                needs_leading_space_check: true,
            };
            return rest;
        }
        bytes
    }

    #[inline]
    fn buffer_line(&mut self, bytes: &[u8]) {
        self.pending_line.extend_from_slice(bytes);
        self.mode = LineMode::Buffered;
    }

    /// # Errors
    /// Returns an error for invalid UTF-8 in a recognized field value or an
    /// unknown field when `strict-fields` is enabled.
    fn finish_buffered_line(&mut self, line: &[u8]) -> Result<Option<Sse>, Error> {
        if matches!(self.mode, LineMode::Start) {
            return self.parse_line(line);
        }
        let mut complete_line = std::mem::take(&mut self.pending_line);
        complete_line.extend_from_slice(line);
        self.mode = LineMode::Start;
        let result = self.parse_line(&complete_line);
        if complete_line.capacity() <= MAX_RETAINED_CAPACITY {
            complete_line.clear();
            self.pending_line = complete_line;
        }
        result
    }

    /// Field rules: <https://html.spec.whatwg.org/multipage/server-sent-events.html#interpreting-an-event-stream>.
    ///
    /// # Errors
    /// Returns an error for invalid UTF-8 in a recognized field value or an
    /// unknown field when `strict-fields` is enabled.
    fn parse_line(&mut self, mut line: &[u8]) -> Result<Option<Sse>, Error> {
        if self.first_line {
            self.first_line = false;
            line = line.strip_prefix(BOM_HEADER).unwrap_or(line);
        }
        if line.is_empty() {
            return Ok(self.event.dispatch());
        }
        let (name, value) = match line.iter().position(|&byte| byte == b':') {
            Some(index) => (&line[..index], &line[index + 1..]),
            None => (line, &b""[..]),
        };
        let value = value.strip_prefix(b" ").unwrap_or(value);
        match name {
            b"data" => self.event.push_data_line(value)?,
            b"event" => {
                self.event.event = Some(validate_utf8(value).map_err(Error::Utf8Parse)?.to_owned());
                self.event.has_fields = true;
            }
            b"id" if !value.contains(&0) => {
                self.event.id = Some(validate_utf8(value).map_err(Error::Utf8Parse)?.to_owned());
                self.event.has_fields = true;
            }
            b"id" => {} // NULL invalidates the value, not the field name.
            b"retry" => {
                let value = validate_utf8(value).map_err(Error::Utf8Parse)?;
                if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
                    if let Ok(retry) = value.parse() {
                        self.event.retry = Some(retry);
                        self.event.has_fields = true;
                    }
                }
            }
            b"" => trace_comment(value),
            #[cfg(feature = "strict-fields")]
            _ => return Err(Error::UnknownField),
            #[cfg(not(feature = "strict-fields"))]
            _ => {}
        }
        Ok(None)
    }

    /// Skip the common empty boundary after a standalone keep-alive comment.
    #[inline]
    fn skip_idle_boundary<'a>(&mut self, bytes: &'a [u8]) -> &'a [u8] {
        if self.event.has_fields {
            return bytes;
        }
        match bytes.first() {
            Some(b'\n') => &bytes[1..],
            Some(b'\r') => self.advance_past_delimiter(bytes, 0),
            _ => bytes,
        }
    }

    /// A trailing CR remembers that the next chunk's leading LF belongs to it.
    #[inline]
    fn advance_past_delimiter<'a>(&mut self, bytes: &'a [u8], line_end: usize) -> &'a [u8] {
        let rest = &bytes[line_end + 1..];
        if bytes[line_end] == b'\r' {
            if let Some(rest) = rest.strip_prefix(b"\n") {
                return rest;
            }
            if rest.is_empty() {
                self.skip_leading_lf = true;
            }
        }
        rest
    }
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
fn trace_comment(_value: &[u8]) {
    #[cfg(feature = "tracing")]
    if comment_tracing_enabled() {
        let value = _value.strip_prefix(b" ").unwrap_or(_value);
        tracing::debug!(comment = %String::from_utf8_lossy(value), "sse comment line");
    }
}
