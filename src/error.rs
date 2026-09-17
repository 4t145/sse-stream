/// A terminal decoding error. After returning this error, the decoder ends.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// An error from the HTTP body or underlying byte stream.
    #[error("body error: {0}")]
    Body(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Invalid UTF-8 in a recognized field value.
    #[error("utf8 parse error: {0}")]
    Utf8Parse(#[source] std::str::Utf8Error),
    /// An unknown field encountered with the `strict-fields` feature enabled.
    #[error("unknown SSE field")]
    UnknownField,
}

/// Metadata that cannot be represented in an SSE event block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// The event type contains a line ending.
    #[error("SSE event type cannot contain CR/LF")]
    InvalidEvent,
    /// The id contains a line ending or NULL.
    #[error("SSE id cannot contain CR/LF/NULL")]
    InvalidId,
}

/// A terminal error from an encoding HTTP body.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BodyError {
    /// An error from the underlying event stream.
    #[error("event stream error: {0}")]
    Stream(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// An event contains metadata that cannot be encoded.
    #[error("SSE encoding error: {0}")]
    Encode(#[source] EncodeError),
}
