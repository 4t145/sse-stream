# SSE Stream

[![Crates.io Version](https://img.shields.io/crates/v/sse-stream)](https://crates.io/crates/sse-stream)
![Release status](https://github.com/4t145/sse-stream/actions/workflows/release.yml/badge.svg)
[![docs.rs](https://img.shields.io/docsrs/sse-stream)](https://docs.rs/sse-stream/latest/sse_stream)

An SSE decoder/encoder for HTTP bodies and byte streams.

`Sse` represents a raw event block, including metadata-only blocks. It does not
inherit ids or retry values across blocks or manage EventSource reconnections.
Unknown fields are ignored by default; fields without a colon have empty values; repeated
metadata fields use the last valid value. `retry` accepts only ASCII digits that
fit in `u64`, and ids containing NULL are ignored.

Recognized field values require valid UTF-8. Input and UTF-8 errors are returned
once, then the decoder ends. Comments are ignored even with DEBUG tracing.
Incomplete final blocks are discarded at EOF. Parser scratch buffers retain at
most 64 KiB each after an event or buffered line completes; this is not an event
size limit.

See the [0.3 migration notes](https://github.com/4t145/sse-stream/blob/master/CHANGELOG.md)
and [developer documentation](https://github.com/4t145/sse-stream/blob/master/docs/README.md).

## Features

| Feature | Default | Description |
| --- | --- | --- |
| `memchr` | ✓ | SIMD-accelerated line-end scanning via [`memchr`](https://crates.io/crates/memchr). Disable for a scalar fallback with one less dependency. |
| `simdutf8` | ✓ | Accelerate UTF-8 validation of long fields. Short fields and validation errors use the standard library; disable to use standard-library validation throughout. |
| `tracing` | | Log comment lines at DEBUG level via [`tracing`](https://crates.io/crates/tracing). |
| `strict-fields` | | Reject unknown field names with `Error::UnknownField` instead of ignoring them. |

Enable strict decoding with `sse-stream = { version = "0.3", features = ["strict-fields"] }`.
Both decoders then return `Error::UnknownField` once and terminate, including for
unknown colonless fields or unknown fields containing invalid UTF-8. Comments,
known colonless fields, ignored invalid `retry` values, and ids containing NULL
keep their usual behavior. Recognized values still require UTF-8 validation
(except ignored NULL-containing ids). Cargo features are unified: enabling
`strict-fields` affects all users of the same resolved crate, not just one decoder.

## Decode
```rust
# use sse_stream::SseStream;
# use http_body_util::Full;
# use bytes::Bytes;
# use futures_util::StreamExt;
const SSE_BODY: &str =
r#"
retry: 1000
event: userconnect
data: {"username": "bobby", "time": "02:33:48"}

data: Here's a system message of some kind that will get used
data: to accomplish some task.
"#;

let body = Full::<Bytes>::from(SSE_BODY);
let mut sse_body = SseStream::new(body);
async {
    while let Some(sse) = sse_body.next().await {
        println!("{:?}", sse.unwrap());
    }
};
```

If the client already provides a stream of byte buffers, use `SseByteStream`
to avoid converting each buffer into an HTTP body frame and back:

```rust
# use std::convert::Infallible;
# use bytes::Bytes;
# use futures_util::StreamExt;
# use sse_stream::SseByteStream;
let bytes = futures_util::stream::iter([
    Ok::<_, Infallible>(Bytes::from_static(b"data: hello\n\n")),
]);
let mut events = SseByteStream::new(bytes);
async {
    while let Some(event) = events.next().await {
        println!("{:?}", event.unwrap());
    }
};
```

## Encode

Every logical data line receives its own `data:` prefix. Empty and trailing lines
are preserved; CRLF and bare CR are normalized to LF. `Sse::encode()` and
`Bytes::try_from(event)` return `EncodeError` if the event type or id contains
CR/LF, or the id contains NULL. Invalid metadata is rejected before any bytes
are returned.

```rust
# use sse_stream::{EncodeError, Sse};
let bytes = Sse::default().event("message").data("hello\nworld").encode()?;
assert_eq!(bytes.as_ref(), b"event: message\ndata: hello\ndata: world\n\n");
assert_eq!(Sse::default().id("invalid\nid").encode(), Err(EncodeError::InvalidId));
# Ok::<(), EncodeError>(())
```

`SseBody` returns `BodyError::Encode` for invalid metadata or `BodyError::Stream`
for an upstream error. Either error is returned once, then the body ends without
sending further events or keep-alives. `KeepAlive::event` validates and encodes
the event during configuration, returning `Result<KeepAlive, EncodeError>`.
Keep-alive comments support multiple lines and prefix each line as a comment.

```rust
# use std::convert::Infallible;
# use futures_util::StreamExt;
# use sse_stream::{Sse, SseBody};

let stream = futures_util::stream::iter([
    Sse::default().event("1").data("....."),
    Sse::default().event("2").data("....."),
    Sse::default().event("3").data("....."),
])
.map(Result::<Sse, Infallible>::Ok);
let body = SseBody::new(stream);
```
