use std::convert::Infallible;

use futures_util::StreamExt;
use sse_stream::{Sse, SseBody, SseStream};

#[test]
fn encoded_fields_keep_their_wire_representation() {
    let cases = [
        (Sse::default(), "\n"),
        (Sse::default().data(""), "data: \n\n"),
        (Sse::default().event("message"), "event: message\n\n"),
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
        assert_eq!(bytes::Bytes::from(event).as_ref(), expected.as_bytes());
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
        let sse = sse.unwrap();
        assert_eq!(sse, sse_sequence[receive_count]);
        receive_count += 1;
    }
}
