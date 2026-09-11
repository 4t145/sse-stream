#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))]

// reference: https://html.spec.whatwg.org/multipage/server-sent-events.html

mod body;
mod stream;

use std::time::Duration;

pub use body::*;
pub use stream::*;

#[derive(Default, Debug, PartialEq, Eq, Hash, Clone)]
pub struct Sse {
    pub event: Option<String>,
    pub data: Option<String>,
    pub id: Option<String>,
    pub retry: Option<u64>,
}

impl Sse {
    pub fn is_event(&self) -> bool {
        self.event.is_some()
    }
    pub fn is_message(&self) -> bool {
        self.event.is_none()
    }
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }
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
        self.retry = Some(retry.as_millis() as u64);
        self
    }
}

impl From<Sse> for bytes::Bytes {
    fn from(val: Sse) -> Self {
        // Include every field prefix and line ending in the initial allocation.
        let retry = val.retry.map(|retry| retry.to_string());
        let capacity = 1
            + val.event.as_ref().map_or(0, |value| 8 + value.len())
            + val.data.as_ref().map_or(0, |value| 7 + value.len())
            + val.id.as_ref().map_or(0, |value| 5 + value.len())
            + retry.as_ref().map_or(0, |value| 8 + value.len());
        let mut bytes = Vec::with_capacity(capacity);
        if let Some(event) = val.event {
            bytes.extend_from_slice(b"event: ");
            bytes.extend_from_slice(event.as_bytes());
            bytes.push(b'\n');
        }
        if let Some(data) = val.data {
            bytes.extend_from_slice(b"data: ");
            bytes.extend_from_slice(data.as_bytes());
            bytes.push(b'\n');
        }
        if let Some(id) = val.id {
            bytes.extend_from_slice(b"id: ");
            bytes.extend_from_slice(id.as_bytes());
            bytes.push(b'\n');
        }
        if let Some(retry) = retry {
            bytes.extend_from_slice(b"retry: ");
            bytes.extend_from_slice(retry.as_bytes());
            bytes.push(b'\n');
        }
        bytes.push(b'\n');
        bytes.into()
    }
}
