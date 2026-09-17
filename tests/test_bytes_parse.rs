use bytes::{Buf, Bytes};
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::{Full, StreamBody};
use sse_stream::{Sse, SseByteStream, SseStream};

#[path = "test_bytes_parse/fields.rs"]
mod fields;
#[path = "test_bytes_parse/fragments.rs"]
mod fragments;
#[path = "test_bytes_parse/input.rs"]
mod input;
#[path = "test_bytes_parse/lifecycle.rs"]
mod lifecycle;

async fn collect_from_full(data: &[u8]) -> Vec<Sse> {
    let body = Full::<Bytes>::from(data.to_vec());
    let mut sse_body = SseStream::new(body);
    let mut out = Vec::new();
    while let Some(sse) = sse_body.next().await {
        out.push(sse.expect("parse error"));
    }
    out
}

async fn collect_from_chunks(chunks: Vec<&'static [u8]>) -> Vec<Sse> {
    let stream = futures_util::stream::iter(
        chunks
            .into_iter()
            .map(|c| Ok::<_, std::convert::Infallible>(Bytes::from_static(c))),
    );
    let mut sse_body = SseByteStream::new(stream);
    let mut out = Vec::new();
    while let Some(sse) = sse_body.next().await {
        out.push(sse.expect("parse error"));
    }
    out
}

fn data_only(s: &str) -> Sse {
    Sse {
        event: None,
        data: Some(s.to_owned()),
        id: None,
        retry: None,
    }
}
