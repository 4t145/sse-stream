# SSE Stream

[![Crates.io Version](https://img.shields.io/crates/v/sse-stream)](https://crates.io/crates/sse-stream)
![Release status](https://github.com/4t145/sse-stream/actions/workflows/release.yml/badge.svg)
[![docs.rs](https://img.shields.io/docsrs/sse-stream)](https://docs.rs/sse-stream/latest/sse_stream)


An SSE decoder/encoder for HTTP bodies and byte streams.


## Features

| Feature | Default | Description |
| --- | --- | --- |
| `memchr` | ✓ | SIMD-accelerated line-end scanning via [`memchr`](https://crates.io/crates/memchr). Disable for a scalar fallback with one less dependency. |
| `simdutf8` | ✓ | Accelerate UTF-8 validation of long fields. Short fields and validation errors use the standard library; disable to use standard-library validation throughout. |
| `tracing` | | Log parser diagnostics (comments, malformed lines) via [`tracing`](https://crates.io/crates/tracing). |

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
