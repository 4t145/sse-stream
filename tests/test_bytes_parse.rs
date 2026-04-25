use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::{Full, StreamBody};
use sse_stream::{Sse, SseStream};

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
    let stream = futures_util::stream::iter(chunks.into_iter().map(|c| {
        Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(c)))
    }));
    let body = StreamBody::new(stream);
    let mut sse_body = SseStream::new(body);
    let mut out = Vec::new();
    while let Some(sse) = sse_body.next().await {
        out.push(sse.expect("parse error"));
    }
    out
}

#[tokio::test]
async fn test_bytes_parse() {
    let bytes = include_bytes!("data/test_stream.sse");
    let body = Full::<Bytes>::from(bytes.to_vec());

    let mut sse_body = sse_stream::SseStream::new(body);
    while let Some(sse) = sse_body.next().await {
        println!("{:?}", sse.unwrap());
    }
}

#[tokio::test]
async fn test_bom_header_at_start() {
    let sse_data = b"\xEF\xBB\xBFdata: hello\n\n";
    let body = Full::<Bytes>::from(sse_data.to_vec());
    let mut sse_body = sse_stream::SseStream::new(body);

    let sse = sse_body
        .next()
        .await
        .expect("Should have one SSE event")
        .unwrap();
    assert_eq!(sse.data, Some("hello".to_string()));
}

#[tokio::test]
async fn test_bom_split_across_chunks() {
    let chunk1 = Bytes::from_static(b"\xEF");
    let chunk2 = Bytes::from_static(b"\xBB\xBFdata: hello\n\n");

    let body = {
        let stream = futures_util::stream::iter(
            [chunk1, chunk2]
                .into_iter()
                .map(|chunk| Ok::<_, std::convert::Infallible>(Frame::data(chunk))),
        );
        StreamBody::new(stream)
    };
    let mut sse_body = sse_stream::SseStream::new(body);

    let sse = sse_body
        .next()
        .await
        .expect("Should have one SSE event")
        .unwrap();
    assert_eq!(sse.data, Some("hello".to_string()));
}
