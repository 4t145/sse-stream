use super::*;

#[tokio::test]
async fn test_data_prefix_split_at_every_byte() {
    let cases: Vec<Vec<&'static [u8]>> = vec![
        vec![b"d", b"ata: hello\n\n"],
        vec![b"da", b"ta: hello\n\n"],
        vec![b"dat", b"a: hello\n\n"],
        vec![b"data", b": hello\n\n"],
        vec![b"data:", b" hello\n\n"],
    ];

    for chunks in cases {
        assert_eq!(collect_from_chunks(chunks).await, vec![data_only("hello")]);
    }
}

#[tokio::test]
async fn test_field_continuations_are_not_new_lines() {
    let input = b"event: data: update\nid: : stream/7\n: data: ignored\ndata: payload\n\n";
    let expected = vec![Sse::default()
        .event("data: update")
        .id(": stream/7")
        .data("payload")];

    for split in 0..=input.len() {
        let actual = collect_from_chunks(vec![&input[..split], b"", &input[split..]]).await;
        assert_eq!(actual, expected, "split={split}");
    }
}

#[tokio::test]
async fn test_split_prefix_ending_at_colon_preserves_space_handling() {
    for tail in ["hello", " hello", ""] {
        let input = format!("data: {tail}\n\n");
        for split in 1..5 {
            let chunks = [
                &input.as_bytes()[..split],
                b"",
                &input.as_bytes()[split..5],
                b"",
                &input.as_bytes()[5..],
            ];
            let stream = futures_util::stream::iter(
                chunks.into_iter().map(Ok::<_, std::convert::Infallible>),
            );
            let mut events = SseByteStream::new(stream);
            assert_eq!(events.next().await.unwrap().unwrap(), data_only(tail));
            assert!(events.next().await.is_none());
        }
    }
}

#[tokio::test]
async fn test_empty_chunk_between_split_crlf() {
    let out = collect_from_chunks(vec![b"data: hello\r", b"", b"\ndata: world\n\n"]).await;
    assert_eq!(out, vec![data_only("hello\nworld")]);
}

#[tokio::test]
async fn test_event_boundaries_preserve_fields_across_every_split() {
    let input = Bytes::from_static(
        concat!(
            "data: 首条🙂\n\n",
            "event: update\nid: stream/7\nretry: 1000\n",
            "data: first\ndata:\ndata: last\n\n",
            "data: before-id\nid: stream/8\n\n",
            ": keepalive\r\n\r\n",
            "data: crlf\r\n\r\n",
            "data:\n\n",
            "data: unfinished"
        )
        .as_bytes(),
    );
    let expected = vec![
        data_only("首条🙂"),
        Sse::default()
            .event("update")
            .id("stream/7")
            .retry(1000)
            .data("first\n\nlast"),
        Sse::default().data("before-id").id("stream/8"),
        data_only("crlf"),
        data_only(""),
    ];

    for split in 0..=input.len() {
        for body in [false, true] {
            let chunks = [input.slice(..split), Bytes::new(), input.slice(split..)];
            let stream = futures_util::stream::iter(
                chunks.into_iter().map(Ok::<_, std::convert::Infallible>),
            );
            let actual: Vec<Sse> = if body {
                SseStream::new(StreamBody::new(stream.map(|chunk| chunk.map(Frame::data))))
                    .map(|event| event.expect("valid event from HTTP body"))
                    .collect()
                    .await
            } else {
                SseByteStream::new(stream)
                    .map(|event| event.expect("valid event from byte stream"))
                    .collect()
                    .await
            };
            assert_eq!(actual, expected, "split={split}, body={body}");
        }
    }
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
async fn test_fragmented_comment_lines() {
    let out = collect_from_chunks(vec![b": ke", b"ep", b"alive\n", b"\n"]).await;
    assert!(out.is_empty());

    let out = collect_from_chunks(vec![b": comm", b"ent\ndata: before\n\n"]).await;
    assert_eq!(out, vec![data_only("before")]);

    let out = collect_from_chunks(vec![b"data: after\n: comm", b"ent\n\n"]).await;
    assert_eq!(out, vec![data_only("after")]);
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

#[tokio::test]
async fn test_fragmented_data_line_split_mid_utf8_char() {
    // The first fragment already carries the `data:` prefix, so the value
    // streams into the data buffer directly; the multi-byte character 你 is
    // split across two fragments and must be validated only once the line
    // completes.
    let out = collect_from_chunks(vec![b"data: \xe4", b"\xbd\xa0 ok\n\n"]).await;
    assert_eq!(out, vec![data_only("你 ok")]);
}

#[tokio::test]
async fn test_fragmented_data_line_five_byte_chunks() {
    // 5-byte fragments engage the direct-append fast path (`data:` fits) and
    // are guaranteed to split some multi-byte characters.
    const PAYLOAD: &str = "data: 你好世界🦀\n\n";
    let out = collect_from_chunks(PAYLOAD.as_bytes().chunks(5).collect()).await;
    assert_eq!(out, vec![data_only("你好世界🦀")]);
}

#[tokio::test]
async fn test_fragmented_data_line_empty_first_fragment() {
    let out = collect_from_chunks(vec![b"data:", b"payload\n\n"]).await;
    assert_eq!(out, vec![data_only("payload")]);
}

#[tokio::test]
async fn test_fragmented_data_line_does_not_leak_into_next_event() {
    // First event goes through the direct-append path, the second through
    // the buffered path (`da` does not carry the full prefix); both must
    // come out intact.
    let out = collect_from_chunks(vec![b"data: first-", b"part2\n\nda", b"ta: second\n\n"]).await;
    assert_eq!(out, vec![data_only("first-part2"), data_only("second")]);
}

#[tokio::test]
async fn test_invalid_utf8_in_fragmented_data_line() {
    let stream = futures_util::stream::iter(
        [&b"data: \xff"[..], b"\xfe\n\n"]
            .into_iter()
            .map(|c| Ok::<_, std::convert::Infallible>(Frame::data(Bytes::from_static(c)))),
    );
    let body = StreamBody::new(stream);
    let mut sse_body = SseStream::new(body);

    let first = sse_body.next().await.expect("stream ended early");
    assert!(
        matches!(first, Err(sse_stream::Error::Utf8Parse(_))),
        "expected Utf8Parse error, got {first:?}"
    );
}
