use std::borrow::Cow;

use bytes::Bytes;

use super::Sse;
use crate::EncodeError;

const EVENT_PREFIX: &[u8] = b"event: ";
const DATA_PREFIX: &[u8] = b"data: ";
const ID_PREFIX: &[u8] = b"id: ";
const RETRY_PREFIX: &[u8] = b"retry: ";
const COMMENT_PREFIX: &[u8] = b": ";

struct TextLines<'a> {
    value: Cow<'a, str>,
    extra_lines: usize,
}

impl<'a> TextLines<'a> {
    #[inline]
    fn new(value: &'a str) -> Self {
        #[cfg(feature = "memchr")]
        let has_newlines = memchr::memchr2(b'\r', b'\n', value.as_bytes()).is_some();
        #[cfg(not(feature = "memchr"))]
        let has_newlines = value.contains(['\r', '\n']);
        if !has_newlines {
            return Self {
                value: Cow::Borrowed(value),
                extra_lines: 0,
            };
        }
        Self::multiline(value)
    }

    #[cold]
    fn multiline(value: &'a str) -> Self {
        let value = if value.contains('\r') {
            Cow::Owned(value.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(value)
        };
        let extra_lines = value.bytes().filter(|&byte| byte == b'\n').count();
        Self { value, extra_lines }
    }

    fn encoded_len(&self, prefix: &[u8]) -> usize {
        self.value.len() + prefix.len() + 1 + self.extra_lines * prefix.len()
    }

    #[inline]
    fn write(&self, output: &mut Vec<u8>, prefix: &[u8]) {
        if self.extra_lines == 0 {
            write_line(output, prefix, &self.value);
        } else {
            self.write_multiline(output, prefix);
        }
    }

    #[cold]
    fn write_multiline(&self, output: &mut Vec<u8>, prefix: &[u8]) {
        // `split` retains trailing empty data, unlike `str::lines`.
        for line in self.value.split('\n') {
            write_line(output, prefix, line);
        }
    }
}

fn write_line(output: &mut Vec<u8>, prefix: &[u8], value: &str) {
    output.extend_from_slice(prefix);
    output.extend_from_slice(value.as_bytes());
    output.push(b'\n');
}

impl Sse {
    /// Encode a block, normalizing data line endings to LF.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] if the event type or id contains CR/LF, or the
    /// id contains NULL. No bytes are returned for an invalid block.
    pub fn encode(self) -> Result<Bytes, EncodeError> {
        Bytes::try_from(self)
    }
}

impl TryFrom<Sse> for Bytes {
    type Error = EncodeError;

    /// Encode a block, normalizing data line endings to LF.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] if the event type or id contains CR/LF, or the
    /// id contains NULL. These bytes cannot be represented in those SSE fields.
    fn try_from(event: Sse) -> Result<Self, Self::Error> {
        if let Some(value) = &event.event {
            if value.contains(['\r', '\n']) {
                return Err(EncodeError::InvalidEvent);
            }
        }
        if let Some(value) = &event.id {
            if value.contains(['\r', '\n', '\0']) {
                return Err(EncodeError::InvalidId);
            }
        }
        let retry = event.retry.map(|value| value.to_string());
        let data = event.data.as_deref().map(TextLines::new);
        let capacity = 1
            + event
                .event
                .as_ref()
                .map_or(0, |value| EVENT_PREFIX.len() + value.len() + 1)
            + data
                .as_ref()
                .map_or(0, |value| value.encoded_len(DATA_PREFIX))
            + event
                .id
                .as_ref()
                .map_or(0, |value| ID_PREFIX.len() + value.len() + 1)
            + retry
                .as_ref()
                .map_or(0, |value| RETRY_PREFIX.len() + value.len() + 1);
        let mut output = Vec::with_capacity(capacity);
        if let Some(value) = &event.event {
            write_line(&mut output, EVENT_PREFIX, value);
        }
        if let Some(value) = data {
            value.write(&mut output, DATA_PREFIX);
        }
        if let Some(value) = &event.id {
            write_line(&mut output, ID_PREFIX, value);
        }
        if let Some(value) = retry {
            write_line(&mut output, RETRY_PREFIX, &value);
        }
        output.push(b'\n');
        Ok(output.into())
    }
}

pub(crate) fn encode_comment(comment: &str) -> Bytes {
    let comment = TextLines::new(comment);
    let mut output = Vec::with_capacity(comment.encoded_len(COMMENT_PREFIX) + 1);
    comment.write(&mut output, COMMENT_PREFIX);
    output.push(b'\n');
    output.into()
}
