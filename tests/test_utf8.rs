use std::convert::Infallible;

use bytes::Bytes;
use futures_util::StreamExt;
use http_body::Frame;
use http_body_util::StreamBody;
use sse_stream::{Error, Sse, SseByteStream, SseStream};

async fn first_event(input: &[u8], width: usize, body: bool) -> Result<Sse, Error> {
    let chunks = futures_util::stream::iter(
        input
            .chunks(width)
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk))),
    );
    if body {
        SseStream::new(StreamBody::new(chunks.map(|chunk| chunk.map(Frame::data))))
            .next()
            .await
            .expect("one event or error")
    } else {
        SseByteStream::new(chunks)
            .next()
            .await
            .expect("one event or error")
    }
}

#[tokio::test]
async fn unicode_fields_across_validation_and_input_boundaries() {
    for padding in [0, 62, 63, 64, 252, 253, 254, 255, 256, 4096] {
        let value = format!("{}中文🙂é", "x".repeat(padding));
        let input =
            format!("\u{feff}event: {value}\r\nid: {value}\r\ndata: {value}\r\ndata: tail\r\n\r\n");
        let expected = Sse::default()
            .event(value.clone())
            .id(value.clone())
            .data(format!("{value}\ntail"));
        for width in [1, 4, 63, 64, 255, 256, 1460, input.len()] {
            for body in [false, true] {
                assert_eq!(
                    first_event(input.as_bytes(), width, body).await.unwrap(),
                    expected,
                    "padding={padding}, width={width}, body={body}"
                );
            }
        }
    }
}

#[tokio::test]
async fn invalid_utf8_keeps_standard_library_error_details() {
    let invalid_sequences: &[&[u8]] = &[
        b"\xff",
        b"\x80",
        b"\xc0\xaf",         // overlong encoding
        b"\xed\xa0\x80",     // surrogate
        b"\xf4\x90\x80\x80", // beyond U+10FFFF
        b"\xe4\xb8",         // truncated character
    ];
    for field in ["data", "event", "id", "retry"] {
        for offset in [0, 63, 64, 255, 256, 4095] {
            for invalid in invalid_sequences {
                // Always long enough to exercise SIMD, including early errors.
                let mut value = vec![b'x'; 8192];
                value.splice(offset..offset + invalid.len(), invalid.iter().copied());
                let expected = std::str::from_utf8(&value).unwrap_err();
                let mut input = format!("{field}: ").into_bytes();
                input.extend_from_slice(&value);
                input.extend_from_slice(b"\r\n\r\n");
                for width in [1, 4, 64, 255, 1460, input.len()] {
                    for body in [false, true] {
                        let Error::Utf8Parse(actual) = first_event(&input, width, body)
                            .await
                            .expect_err("invalid field must fail UTF-8 validation")
                        else {
                            panic!("expected a UTF-8 error");
                        };
                        assert_eq!(actual.valid_up_to(), expected.valid_up_to());
                        assert_eq!(actual.error_len(), expected.error_len());
                    }
                }
            }
        }
    }

    // A truncated sequence at the end must retain error_len() == None.
    let mut value = vec![b'x'; 8192];
    value.extend_from_slice(b"\xf0\x9f\x99");
    let mut input = b"data: ".to_vec();
    input.extend_from_slice(&value);
    input.extend_from_slice(b"\n\n");
    for width in [1, 255, 1460, input.len()] {
        let Error::Utf8Parse(error) = first_event(&input, width, false).await.unwrap_err() else {
            panic!("expected a UTF-8 error");
        };
        assert_eq!(error.valid_up_to(), 8192);
        assert_eq!(error.error_len(), None);
    }
}

#[tokio::test]
async fn long_retry_whitespace_is_ignored() {
    let input = format!(
        "retry: {}18446744073709551615{}\ndata: after\n\n",
        " ".repeat(256),
        " ".repeat(256)
    );
    for width in [1, 255, input.len()] {
        assert_eq!(
            first_event(input.as_bytes(), width, false).await.unwrap(),
            Sse::default().data("after")
        );
    }
}
