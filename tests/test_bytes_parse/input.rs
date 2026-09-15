use super::*;

struct ChainedFrameBody {
    sent: bool,
    first: &'static [u8],
    second: &'static [u8],
}

impl http_body::Body for ChainedFrameBody {
    type Data = bytes::buf::Chain<Bytes, Bytes>;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.sent {
            return std::task::Poll::Ready(None);
        }
        self.sent = true;
        let chained = bytes::Buf::chain(
            Bytes::from_static(self.first),
            Bytes::from_static(self.second),
        );
        std::task::Poll::Ready(Some(Ok(Frame::data(chained))))
    }
}

#[tokio::test]
async fn test_multi_segment_buf_frame_not_truncated() {
    // The full frame, once flattened, is a single complete SSE event.
    // If `chunk()` is used naively, only the first segment ("data: hel")
    // is read and the message is silently dropped at end-of-stream.
    let body = ChainedFrameBody {
        sent: false,
        first: b"data: hel",
        second: b"lo\n\n",
    };
    let mut sse_body = SseStream::new(body);
    let mut out = Vec::new();
    while let Some(sse) = sse_body.next().await {
        out.push(sse.expect("parse error"));
    }
    assert_eq!(
        out,
        vec![Sse {
            event: None,
            data: Some("hello".into()),
            id: None,
            retry: None,
        }],
        "multi-segment Buf frame must be fully consumed"
    );
}

#[tokio::test]
async fn test_multi_segment_buf_from_byte_stream_not_truncated() {
    let data = Bytes::from_static(b"data: hel").chain(Bytes::from_static(b"lo\n\n"));
    let stream = futures_util::stream::iter([Ok::<_, std::convert::Infallible>(data)]);
    let mut events = SseByteStream::new(stream);

    assert_eq!(events.next().await.unwrap().unwrap(), data_only("hello"));
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn test_multiple_events_retained_from_one_byte_buffer() {
    let data = Bytes::from_static(b"data: one\n\ndata: two\n\n");
    let stream =
        futures_util::stream::iter([Ok(data), Err(std::io::Error::other("end-of-test error"))]);
    let mut events = SseByteStream::new(stream);

    assert_eq!(events.next().await.unwrap().unwrap(), data_only("one"));
    assert_eq!(events.next().await.unwrap().unwrap(), data_only("two"));
    assert!(matches!(
        events.next().await,
        Some(Err(sse_stream::Error::Body(_)))
    ));
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn test_multiple_events_retained_from_one_body_frame() {
    let body = Full::new(Bytes::from_static(b"data: one\n\ndata: two\n\n"));
    let mut events = SseStream::new(body);

    assert_eq!(events.next().await.unwrap().unwrap(), data_only("one"));
    assert_eq!(events.next().await.unwrap().unwrap(), data_only("two"));
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn test_event_split_across_many_immediately_ready_fragments() {
    const SEGMENTS: usize = 10_000;
    let mut fragments: Vec<&'static [u8]> = Vec::with_capacity(SEGMENTS + 2);
    fragments.push(b"data: ");
    fragments.extend((0..SEGMENTS).map(|_| b"x".as_slice()));
    fragments.push(b"\n\n");

    let events = collect_from_chunks(fragments).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.as_deref().map(str::len), Some(SEGMENTS));
}

#[tokio::test]
async fn test_bom_split_across_segments_of_one_frame() {
    let data = Bytes::from_static(b"\xEF")
        .chain(Bytes::from_static(b"\xBB"))
        .chain(Bytes::from_static(b"\xBF"))
        .chain(Bytes::from_static(b"data: hello\n\n"));
    let mut events = SseStream::new(Full::new(data));

    assert_eq!(events.next().await.unwrap().unwrap(), data_only("hello"),);
    assert!(events.next().await.is_none());
}

#[tokio::test]
async fn test_crlf_split_across_segments_of_one_frame() {
    let data = Bytes::from_static(b"data: hello\r")
        .chain(Bytes::from_static(b"\n"))
        .chain(Bytes::from_static(b"data: world\n\n"));
    let mut events = SseStream::new(Full::new(data));

    assert_eq!(
        events.next().await.unwrap().unwrap(),
        data_only("hello\nworld"),
    );
    assert!(events.next().await.is_none());
}
