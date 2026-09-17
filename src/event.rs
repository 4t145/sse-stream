use std::time::Duration;

pub(crate) mod encode;

/// A raw SSE event block. Fields are not inherited from previous blocks.
///
/// Metadata-only blocks are returned too; this type does not implement the
/// browser EventSource connection or reconnection state machine.
#[derive(Default, Debug, PartialEq, Eq, Hash, Clone)]
pub struct Sse {
    /// Explicit event type. Must not contain CR/LF when encoded.
    pub event: Option<String>,
    /// Data joined with LF; `None` and an empty data field are distinct.
    pub data: Option<String>,
    /// Explicit id. Must not contain CR/LF/NULL when encoded.
    pub id: Option<String>,
    /// Explicit reconnection delay, in milliseconds.
    pub retry: Option<u64>,
}

impl Sse {
    /// Whether the block contains an explicit event field (including an empty one).
    pub fn is_event(&self) -> bool {
        self.event.is_some()
    }
    /// Whether the block has no explicit event field.
    pub fn is_message(&self) -> bool {
        self.event.is_none()
    }
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }
    /// Set data. Encoding normalizes CRLF and CR to LF, preserving empty lines.
    pub fn data(mut self, data: impl Into<String>) -> Self {
        self.data = Some(data.into());
        self
    }
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn retry(mut self, retry: u64) -> Self {
        self.retry = Some(retry);
        self
    }
    pub fn retry_duration(mut self, retry: Duration) -> Self {
        self.retry = Some(retry.as_millis().min(u64::MAX as u128) as u64);
        self
    }
}
