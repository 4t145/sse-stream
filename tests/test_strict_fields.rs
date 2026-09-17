use std::convert::Infallible;

use bytes::Bytes;
use futures_util::{stream::FusedStream, Stream, StreamExt};
use http_body::Frame;
use http_body_util::StreamBody;
use sse_stream::{Error, Sse, SseByteStream, SseStream};

async fn check_stream<S>(mut stream: S, unknown: bool, expected: &[Sse])
where
    S: Stream<Item = Result<Sse, Error>> + FusedStream + Unpin,
{
    for event in expected {
        assert_eq!(stream.next().await.unwrap().unwrap(), *event);
    }
    if unknown && cfg!(feature = "strict-fields") {
        assert!(matches!(
            stream.next().await,
            Some(Err(Error::UnknownField))
        ));
    }
    for _ in 0..3 {
        assert!(
            stream.next().await.is_none(),
            "decoder must stay terminated"
        );
        assert!(stream.is_terminated());
    }
}

async fn check_decoders(input: &[u8], unknown: bool, expected: &[Sse]) {
    // Width 1 splits the BOM, CRLF, field names, and values; larger widths
    // also exercise complete lines and multiple events in the same buffer.
    for width in 1..=input.len() {
        let chunks = || {
            futures_util::stream::iter(
                input
                    .chunks(width)
                    .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk))),
            )
        };
        check_stream(SseByteStream::new(chunks()), unknown, expected).await;
        check_stream(
            SseStream::new(StreamBody::new(
                chunks().map(|chunk| chunk.map(Frame::data)),
            )),
            unknown,
            expected,
        )
        .await;
    }
}

#[tokio::test]
async fn unknown_fields_follow_feature_policy_and_errors_are_terminal() {
    let lines: &[&[u8]] = &[
        b"unknown: extension",
        b"unknown",
        b"Data: case sensitive",
        b" data: leading space",
        b"\xff: invalid name",
        b"\xff",
        b"unknown: \xff",
        b"unknown:\0\xff",
    ];
    for line in lines {
        let mut input = b"\xef\xbb\xbfdata: before\r\n\r\ndata: partial\r\n".to_vec();
        input.extend_from_slice(line);
        // A strict error discards the partially built block and prevents
        // both buffered and future input from producing further events.
        input.extend_from_slice(b"\r\ndata: after\r\n\r\ndata: later\r\n\r\n");
        let mut expected = vec![Sse::default().data("before")];
        if !cfg!(feature = "strict-fields") {
            expected.push(Sse::default().data("partial\nafter"));
            expected.push(Sse::default().data("later"));
        }
        check_decoders(&input, true, &expected).await;

        // Unknown fields alone must not manufacture empty events by default.
        let mut input = b"\xef\xbb\xbf".to_vec();
        input.extend_from_slice(line);
        input.extend_from_slice(b"\r\n\r\n");
        check_decoders(&input, true, &[]).await;
    }
}

#[tokio::test]
async fn comments_and_known_fields_remain_valid_in_strict_mode() {
    let input = b"\xef\xbb\xbf: \xff\r\n:\r\n\r\n\
        event: old\r\nevent\r\nid: old\r\nid\r\nid: bad\0\xff\r\n\
        retry: 12\r\nretry\r\nretry: nope\r\nretry: +56\r\nretry: 78 \r\n\
        retry: 18446744073709551616\r\ndata\r\ndata: tail\r\n\r\n\
        retry: nope\r\nid: \0\r\n\r\n";
    check_decoders(
        input,
        false,
        &[Sse::default().event("").id("").retry(12).data("\ntail")],
    )
    .await;
}

#[tokio::test]
async fn invalid_utf8_in_known_values_is_not_an_unknown_field() {
    for field in ["data", "event", "id", "retry"] {
        let input = format!("{field}: ").into_bytes();
        let input = [input.as_slice(), b"\xff\r\n\r\n"].concat();
        for width in 1..=input.len() {
            let chunks = || {
                futures_util::stream::iter(
                    input
                        .chunks(width)
                        .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk))),
                )
            };
            let mut bytes = SseByteStream::new(chunks());
            assert!(matches!(bytes.next().await, Some(Err(Error::Utf8Parse(_)))));
            check_stream(bytes, false, &[]).await;
            let mut body = SseStream::new(StreamBody::new(
                chunks().map(|chunk| chunk.map(Frame::data)),
            ));
            assert!(matches!(body.next().await, Some(Err(Error::Utf8Parse(_)))));
            check_stream(body, false, &[]).await;
        }
    }
}
