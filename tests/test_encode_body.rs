use std::{convert::Infallible, error::Error as _};

use futures_util::StreamExt;
use sse_stream::{BodyError, EncodeError, KeepAlive, Sse, SseBody, SseStream};

#[test]
fn encoded_fields_keep_their_wire_representation() {
    let cases = [
        (Sse::default(), "\n"),
        (Sse::default().data(""), "data: \n\n"),
        (Sse::default().event("message"), "event: message\n\n"),
        (Sse::default().event("x\0y"), "event: x\0y\n\n"),
        (Sse::default().id(""), "id: \n\n"),
        (Sse::default().retry(0), "retry: 0\n\n"),
        (
            Sse::default().retry(u64::MAX),
            "retry: 18446744073709551615\n\n",
        ),
        (
            Sse::default()
                .event("message")
                .data(r#"{"text":"中文🙂\nnext"}"#)
                .id("abc")
                .retry(1000),
            "event: message\ndata: {\"text\":\"中文🙂\\nnext\"}\nid: abc\nretry: 1000\n\n",
        ),
    ];
    for (event, expected) in cases {
        assert_eq!(
            bytes::Bytes::try_from(event)
                .expect("valid metadata")
                .as_ref(),
            expected.as_bytes()
        );
    }
}

#[tokio::test]
async fn test_encode_body() {
    let sse_sequence = [
        Sse::default().event("1").data("....."),
        Sse::default().event("2").data("....."),
        Sse::default().event("3").data("....."),
        Sse::default().event("4").data("....."),
    ];
    let stream =
        futures_util::stream::iter(sse_sequence.clone()).map(Result::<Sse, Infallible>::Ok);
    let body = SseBody::new(stream);
    let mut stream = SseStream::new(body);
    let mut receive_count = 0;
    while let Some(sse) = stream.next().await {
        let sse = sse.expect("valid encoded event");
        assert_eq!(sse, sse_sequence[receive_count]);
        receive_count += 1;
    }
    assert_eq!(receive_count, sse_sequence.len());
}

#[tokio::test]
async fn body_preserves_boxed_upstream_errors() {
    let source: Box<dyn std::error::Error + Send + Sync> = Box::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "source closed",
    ));
    let input = futures_util::stream::iter([Err::<Sse, _>(source)]);
    let mut decoded = SseStream::new(SseBody::new(input));
    let error = decoded
        .next()
        .await
        .expect("body failure")
        .expect_err("upstream error");
    let body_error = error
        .source()
        .and_then(|source| source.downcast_ref::<BodyError>())
        .expect("HTTP body error");
    let BodyError::Stream(source) = body_error else {
        panic!("expected upstream error");
    };
    let source = source
        .downcast_ref::<std::io::Error>()
        .expect("original error type");
    assert_eq!(source.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(source.to_string(), "source closed");
    assert!(decoded.next().await.is_none());
}

#[tokio::test]
async fn multiline_data_preserves_empty_lines_and_normalizes_line_endings() {
    for (value, normalized) in [
        ("a\nb", "a\nb"),
        ("a\n\nb", "a\n\nb"),
        ("\na\n", "\na\n"),
        ("\n", "\n"),
        ("\n\n", "\n\n"),
        ("a\r\nb\r", "a\nb\n"),
        ("a\rb", "a\nb"),
        ("\r\n", "\n"),
        ("a\n\r\nb", "a\n\nb"),
        (
            "中文🙂\n data: x\n\nid: injected",
            "中文🙂\n data: x\n\nid: injected",
        ),
    ] {
        let event = Sse::default().id("original").event("message").data(value);
        let wire = event.encode().expect("valid metadata");
        for width in [1, 4, wire.len()] {
            let chunks = futures_util::stream::iter(wire.chunks(width).map(Ok::<_, Infallible>));
            let mut events = sse_stream::SseByteStream::new(chunks);
            let decoded = events
                .next()
                .await
                .expect("encoded event")
                .expect("valid wire");
            assert_eq!(
                decoded,
                Sse::default()
                    .id("original")
                    .event("message")
                    .data(normalized)
            );
            assert!(events.next().await.is_none());
        }
    }
}

#[test]
fn invalid_metadata_is_rejected_at_encoding_boundary() {
    for value in ["x\ny", "x\ry", "x\r\ny"] {
        assert_eq!(
            Sse::default().event(value).encode(),
            Err(EncodeError::InvalidEvent)
        );
        assert_eq!(
            bytes::Bytes::try_from(Sse::default().id(value)),
            Err(EncodeError::InvalidId)
        );
    }
    assert_eq!(
        Sse {
            id: Some("x\0y".into()),
            ..Sse::default()
        }
        .encode(),
        Err(EncodeError::InvalidId)
    );
}

#[test]
fn keep_alive_rejects_invalid_metadata_during_configuration() {
    for (event, expected) in [
        (Sse::default().event("x\ny"), EncodeError::InvalidEvent),
        (Sse::default().id("x\ry"), EncodeError::InvalidId),
        (Sse::default().id("x\0y"), EncodeError::InvalidId),
    ] {
        assert_eq!(
            KeepAlive::new().event(event).expect_err("invalid metadata"),
            expected
        );
    }
}

#[test]
fn retry_duration_saturates_instead_of_wrapping() {
    assert_eq!(
        Sse::default()
            .retry_duration(std::time::Duration::MAX)
            .retry,
        Some(u64::MAX)
    );
}
