use super::*;

#[tokio::test]
async fn test_bytes_parse() {
    let bytes = include_bytes!("assets/test_stream.sse");
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
async fn field_rules_are_independent_of_fragmentation() {
    let input = concat!(
        "unknown: extension\nunknown\n",
        "event: old\nevent: new\n",
        "id: old\nid: new\nid: bad\0id\n",
        "retry: 12\nretry: 34\nretry: +56\nretry: 78 \n",
        "retry: 18446744073709551616\nretry\n",
        "data\ndata: tail\n\n",
        "id\nevent\n\n",
        "retry: 0007\n\n",
        "data: next\n\n",
    );
    let expected = vec![
        Sse::default()
            .event("new")
            .id("new")
            .retry(34)
            .data("\ntail"),
        Sse::default().id("").event(""),
        Sse::default().retry(7),
        Sse::default().data("next"),
    ];
    for width in 1..=input.len() {
        let actual = collect_from_chunks(input.as_bytes().chunks(width).collect()).await;
        assert_eq!(actual, expected, "width={width}");
    }
}

#[tokio::test]
async fn ignored_fields_do_not_create_events() {
    assert!(collect_from_full(b"unknown: x\nretry: nope\nid: \0\n\n")
        .await
        .is_empty());
    let input = b": \xff\r\n\r\ndata: after\n\n";
    for width in 1..=input.len() {
        assert_eq!(
            collect_from_chunks(input.chunks(width).collect()).await,
            vec![data_only("after")]
        );
    }
}
