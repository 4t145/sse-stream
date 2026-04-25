use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::{Full, StreamBody};
use sse_stream::{Sse, SseStream};

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
            .map(|c| Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(c)))),
    );
    let body = StreamBody::new(stream);
    let mut sse_body = SseStream::new(body);
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
async fn test_line_break_crlf() {
    let out = collect_from_full(b"data: a\r\ndata: b\r\n\r\n").await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

#[tokio::test]
async fn test_line_break_cr_only() {
    let out = collect_from_full(b"data: a\rdata: b\r\r").await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

#[tokio::test]
async fn test_line_break_mixed() {
    // `\n`, `\r`, and `\r\n` interleaved.
    let payload: &[u8] = b"data: one\r\ndata: two\rdata: three\n\r\n";
    let out = collect_from_full(payload).await;
    assert_eq!(out, vec![data_only("one\ntwo\nthree")]);
}

#[tokio::test]
async fn test_cr_lf_split_across_chunks() {
    // Original payload:  "data: hello\r\ndata: world\n\n"
    // Split:             "data: hello\r"  +  "\ndata: world\n\n"
    let out = collect_from_chunks(vec![b"data: hello\r", b"\ndata: world\n\n"]).await;
    assert_eq!(out, vec![data_only("hello\nworld")]);
}

#[tokio::test]
async fn test_cr_then_non_lf_across_chunks() {
    // Original: "data: a\rdata: b\n\n"  =>  one event with "a\nb"
    let out = collect_from_chunks(vec![b"data: a\r", b"data: b\n\n"]).await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

#[tokio::test]
async fn test_cr_then_cr_across_chunks() {
    // Original: "data: a\r\rdata: b\n\n" -> ["data: a", "", "data: b", ""]
    // -> dispatch event {data:"a"} on the second "", then "data: b" continues a new event
    let out = collect_from_chunks(vec![b"data: a\r", b"\rdata: b\n\n"]).await;
    assert_eq!(out, vec![data_only("a"), data_only("b")]);
}

#[tokio::test]
async fn test_dispatch_boundary_split_at_cr() {
    // "data: x\r\n\r\n" split as "data: x\r\n\r" + "\n"
    // Expected: one event {data: "x"}.
    let out = collect_from_chunks(vec![b"data: x\r\n\r", b"\n"]).await;
    assert_eq!(out, vec![data_only("x")]);
}

#[tokio::test]
async fn test_multiple_consecutive_cr() {
    // "data: a\r\r\r" -> lines: "data: a", "", ""  -> dispatch after first empty.
    let out = collect_from_full(b"data: a\r\r\r").await;
    assert_eq!(out, vec![data_only("a")]);
}

#[tokio::test]
async fn test_comment_lines() {
    let out = collect_from_full(b": this is a comment\ndata: hi\n: another\n\n").await;
    assert_eq!(out, vec![data_only("hi")]);
}

#[tokio::test]
async fn test_empty_data_field() {
    let out = collect_from_full(b"data:\n\n").await;
    assert_eq!(out, vec![data_only("")]);
}

#[tokio::test]
async fn test_two_empty_data_lines_join_with_newline() {
    let out = collect_from_full(b"data:\ndata:\n\n").await;
    assert_eq!(out, vec![data_only("\n")]);
}

#[tokio::test]
async fn test_only_one_leading_space_stripped() {
    let out = collect_from_full(b"data:  hello\n\n").await;
    // The first space is stripped, the second is preserved.
    assert_eq!(out, vec![data_only(" hello")]);
}

#[tokio::test]
async fn test_id_with_null_byte_ignored() {
    let payload: &[u8] = b"id: ab\x00cd\ndata: x\n\n";
    let stream = futures_util::stream::iter(std::iter::once(Ok::<_, std::convert::Infallible>(
        Frame::data(Bytes::from_static(payload)),
    )));
    let body = StreamBody::new(stream);
    let mut sse_body = SseStream::new(body);
    let mut out = Vec::new();
    while let Some(sse) = sse_body.next().await {
        out.push(sse.expect("parse error"));
    }
    assert_eq!(out, vec![data_only("x")], "id with NULL must be ignored");
}

#[tokio::test]
async fn test_incomplete_trailing_event_discarded() {
    // No empty line after the second event.
    let out = collect_from_full(b"data: complete\n\ndata: incomplete\n").await;
    assert_eq!(out, vec![data_only("complete")]);
}

#[tokio::test]
async fn test_multiple_events_split_chunks() {
    let out = collect_from_chunks(vec![
        b"event: a\ndata: 1\n",
        b"\nevent: b\nda",
        b"ta: 2\n\n",
    ])
    .await;
    assert_eq!(
        out,
        vec![
            Sse {
                event: Some("a".into()),
                data: Some("1".into()),
                ..Default::default()
            },
            Sse {
                event: Some("b".into()),
                data: Some("2".into()),
                ..Default::default()
            },
        ]
    );
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
