# SSE Stream

[![Crates.io Version](https://img.shields.io/crates/v/sse-stream)](https://crates.io/crates/sse-stream)
![Release status](https://github.com/4t145/sse-stream/actions/workflows/release.yml/badge.svg)
[![docs.rs](https://img.shields.io/docsrs/sse-stream)](https://docs.rs/sse-stream/latest/sse-stream)


A SSE decoder for Http body

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
