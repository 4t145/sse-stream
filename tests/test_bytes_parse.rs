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

// =====================================================================
// Line-break handling corner cases
// =====================================================================

/// `\n` line break only.
#[tokio::test]
async fn test_line_break_lf_only() {
    let out = collect_from_full(b"data: a\ndata: b\n\n").await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

/// `\r\n` line break (Windows-style).
#[tokio::test]
async fn test_line_break_crlf() {
    let out = collect_from_full(b"data: a\r\ndata: b\r\n\r\n").await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

/// Bare `\r` line break (legacy Mac style).
#[tokio::test]
async fn test_line_break_cr_only() {
    let out = collect_from_full(b"data: a\rdata: b\r\r").await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

/// Mixed line breaks within the same payload.
#[tokio::test]
async fn test_line_break_mixed() {
    // `\n`, `\r`, and `\r\n` interleaved.
    let payload: &[u8] = b"data: one\r\ndata: two\rdata: three\n\r\n";
    let out = collect_from_full(payload).await;
    assert_eq!(out, vec![data_only("one\ntwo\nthree")]);
}

/// `\r` at the END of one chunk and `\n` at the START of the next chunk
/// must be treated as ONE `\r\n` line break, not two empty separator lines.
/// This is the central bug `MayTrailingNewline` is meant to fix.
#[tokio::test]
async fn test_cr_lf_split_across_chunks() {
    // Original payload:  "data: hello\r\ndata: world\n\n"
    // Split:             "data: hello\r"  +  "\ndata: world\n\n"
    let out = collect_from_chunks(vec![b"data: hello\r", b"\ndata: world\n\n"]).await;
    assert_eq!(out, vec![data_only("hello\nworld")]);
}

/// Trailing `\r` followed by a non-`\n` char in the next chunk should
/// produce TWO line breaks (the `\r` alone, then a fresh line).
#[tokio::test]
async fn test_cr_then_non_lf_across_chunks() {
    // Original: "data: a\rdata: b\n\n"  =>  one event with "a\nb"
    let out = collect_from_chunks(vec![b"data: a\r", b"data: b\n\n"]).await;
    assert_eq!(out, vec![data_only("a\nb")]);
}

/// Trailing `\r` followed by another `\r` across chunks should produce
/// two distinct line terminators (and thus an empty-line dispatch in between).
#[tokio::test]
async fn test_cr_then_cr_across_chunks() {
    // Original: "data: a\r\rdata: b\n\n" -> ["data: a", "", "data: b", ""]
    // -> dispatch event {data:"a"} on the second "", then "data: b" continues a new event
    let out = collect_from_chunks(vec![b"data: a\r", b"\rdata: b\n\n"]).await;
    assert_eq!(out, vec![data_only("a"), data_only("b")]);
}

/// Pathological: every byte arrives in its own chunk.
#[tokio::test]
async fn test_one_byte_per_chunk() {
    let payload: &[u8] = b"data: hi\r\n\r\n";
    let chunks: Vec<&'static [u8]> = vec![
        b"d", b"a", b"t", b"a", b":", b" ", b"h", b"i", b"\r", b"\n", b"\r", b"\n",
    ];
    // sanity check
    let joined: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();
    assert_eq!(joined, payload);

    let out = collect_from_chunks(chunks).await;
    assert_eq!(out, vec![data_only("hi")]);
}

/// The dispatch boundary (the empty line) is split exactly at the `\r`.
#[tokio::test]
async fn test_dispatch_boundary_split_at_cr() {
    // "data: x\r\n\r\n" split as "data: x\r\n\r" + "\n"
    // Expected: one event {data: "x"}.
    let out = collect_from_chunks(vec![b"data: x\r\n\r", b"\n"]).await;
    assert_eq!(out, vec![data_only("x")]);
}

/// Multiple consecutive `\r`s should produce multiple line terminators.
#[tokio::test]
async fn test_multiple_consecutive_cr() {
    // "data: a\r\r\r" -> lines: "data: a", "", ""  -> dispatch after first empty.
    let out = collect_from_full(b"data: a\r\r\r").await;
    assert_eq!(out, vec![data_only("a")]);
}

// =====================================================================
// BOM handling combined with `\r` line breaks
// =====================================================================

#[tokio::test]
async fn test_bom_with_cr_line_breaks() {
    let out = collect_from_full(b"\xEF\xBB\xBFdata: hello\r\r").await;
    assert_eq!(out, vec![data_only("hello")]);
}

#[tokio::test]
async fn test_bom_split_then_cr_split() {
    // BOM split, AND the trailing `\r\n` split across chunks.
    let out = collect_from_chunks(vec![
        b"\xEF\xBB",
        b"\xBFdata: hello\r",
        b"\n\r\n",
    ])
    .await;
    assert_eq!(out, vec![data_only("hello")]);
}

// =====================================================================
// Field parsing corner cases (post-fix: relaxed semantics)
// =====================================================================

/// Per the SSE spec, a line with no `:` is treated as a field name with
/// empty value. The fix changed this from an error to a silent ignore
/// (since none of the recognised fields can have an empty name).
#[tokio::test]
async fn test_line_without_colon_is_ignored() {
    let out = collect_from_full(b"foobar\ndata: hi\n\n").await;
    assert_eq!(out, vec![data_only("hi")]);
}

/// Comment lines (starting with `:`) must be ignored but must NOT break dispatch.
#[tokio::test]
async fn test_comment_lines() {
    let out = collect_from_full(b": this is a comment\ndata: hi\n: another\n\n").await;
    assert_eq!(out, vec![data_only("hi")]);
}

/// `data:` with no value should produce an empty data string.
#[tokio::test]
async fn test_empty_data_field() {
    let out = collect_from_full(b"data:\n\n").await;
    assert_eq!(out, vec![data_only("")]);
}

/// `data: ` -> only the single leading space stripped, value is empty.
/// Two consecutive `data:` lines produce a single `\n`.
#[tokio::test]
async fn test_two_empty_data_lines_join_with_newline() {
    let out = collect_from_full(b"data:\ndata:\n\n").await;
    assert_eq!(out, vec![data_only("\n")]);
}

/// Only a single leading space is stripped from field values.
#[tokio::test]
async fn test_only_one_leading_space_stripped() {
    let out = collect_from_full(b"data:  hello\n\n").await;
    // The first space is stripped, the second is preserved.
    assert_eq!(out, vec![data_only(" hello")]);
}

/// Duplicated `event`/`id` lines should now overwrite (not error).
#[tokio::test]
async fn test_duplicated_event_overwrites() {
    let out = collect_from_full(b"event: first\nevent: second\ndata: x\n\n").await;
    assert_eq!(
        out,
        vec![Sse {
            event: Some("second".into()),
            data: Some("x".into()),
            id: None,
            retry: None,
        }]
    );
}

#[tokio::test]
async fn test_duplicated_id_overwrites() {
    let out = collect_from_full(b"id: a\nid: b\ndata: x\n\n").await;
    assert_eq!(
        out,
        vec![Sse {
            event: None,
            data: Some("x".into()),
            id: Some("b".into()),
            retry: None,
        }]
    );
}

/// Invalid `retry:` value (non-int) should be silently ignored, not error.
#[tokio::test]
async fn test_invalid_retry_ignored() {
    let out = collect_from_full(b"retry: not-a-number\ndata: x\n\n").await;
    assert_eq!(out, vec![data_only("x")]);
}

/// Unknown fields should be ignored, not error.
#[tokio::test]
async fn test_unknown_field_ignored() {
    let out = collect_from_full(b"weird: field\ndata: x\n\n").await;
    assert_eq!(out, vec![data_only("x")]);
}

/// Stream that ends without a terminating empty line: the trailing
/// incomplete event MUST be discarded (per the post-fix `None` branch).
#[tokio::test]
async fn test_incomplete_trailing_event_discarded() {
    // No empty line after the second event.
    let out = collect_from_full(b"data: complete\n\ndata: incomplete\n").await;
    assert_eq!(out, vec![data_only("complete")]);
}

/// Single-event payload split right inside a UTF-8 multi-byte character
/// inside the data field (across chunks). Buffering must keep it intact.
#[tokio::test]
async fn test_multibyte_split_across_chunks() {
    // "中" is 0xE4 0xB8 0xAD in UTF-8.
    let out = collect_from_chunks(vec![b"data: \xE4", b"\xB8\xAD\n\n"]).await;
    assert_eq!(out, vec![data_only("中")]);
}

/// Multiple complete events split awkwardly across chunks.
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
