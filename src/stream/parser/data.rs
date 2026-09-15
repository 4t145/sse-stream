use crate::Error;

use super::scan::validate_utf8;

/// Bounds idle scratch space after an event or a buffered line completes.
pub(super) const MAX_RETAINED_CAPACITY: usize = 64 * 1024;
const COMPACT_OUTPUT_THRESHOLD: usize = 4096;
const COMPACT_OUTPUT_RATIO: usize = 4;

#[derive(Default)]
pub(super) struct DataBuffer {
    bytes: Vec<u8>,
    /// Validated, newline-terminated lines occupy exactly this prefix.
    validated_len: usize,
}

impl DataBuffer {
    /// # Errors
    /// Returns a UTF-8 error for an invalid field value.
    #[inline]
    pub(super) fn push_line(&mut self, value: &[u8]) -> Result<(), Error> {
        let value = validate_utf8(value).map_err(Error::Utf8Parse)?;
        self.bytes.truncate(self.validated_len);
        self.bytes.extend_from_slice(value.as_bytes());
        self.bytes.push(b'\n');
        self.validated_len = self.bytes.len();
        Ok(())
    }

    #[inline]
    pub(super) fn push_fragment(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// # Errors
    /// Returns a UTF-8 error and discards the unfinished line if invalid.
    #[inline]
    pub(super) fn finish_line(&mut self) -> Result<(), Error> {
        if let Err(error) = validate_utf8(&self.bytes[self.validated_len..]) {
            self.bytes.truncate(self.validated_len);
            return Err(Error::Utf8Parse(error));
        }
        self.bytes.push(b'\n');
        self.validated_len = self.bytes.len();
        Ok(())
    }

    #[inline]
    pub(super) fn take(&mut self) -> Option<String> {
        // Never expose an unfinished suffix, even if dispatch is called early.
        self.bytes.truncate(self.validated_len);
        if self.validated_len == 0 {
            return None;
        }
        self.validated_len = 0;
        self.bytes.pop();
        let output = if self.bytes.capacity() > COMPACT_OUTPUT_THRESHOLD
            && self.bytes.len() < self.bytes.capacity() / COMPACT_OUTPUT_RATIO
        {
            let output = self.bytes.clone();
            self.bytes.clear();
            if self.bytes.capacity() > MAX_RETAINED_CAPACITY {
                self.bytes = Vec::with_capacity(MAX_RETAINED_CAPACITY);
            }
            output
        } else {
            let output = std::mem::take(&mut self.bytes);
            self.bytes = Vec::with_capacity(output.capacity().min(MAX_RETAINED_CAPACITY));
            output
        };
        // SAFETY: Only push_line/finish_line extend validated_len, after UTF-8
        // validation. The suffix was truncated above, and popping the final
        // ASCII newline preserves UTF-8. The byte vector is private to this type.
        Some(unsafe { String::from_utf8_unchecked(output) })
    }
}

#[cfg(test)]
mod tests;
