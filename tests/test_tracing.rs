#![cfg(feature = "tracing")]

// Isolate debug tracing from parallel parser tests with no subscriber.
use bytes::Bytes;
use futures_util::StreamExt;
use sse_stream::{Sse, SseByteStream};

#[tokio::test]
async fn fragmented_invalid_comment_is_ignored_with_debug_tracing() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::sink)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    assert!(tracing::enabled!(tracing::Level::DEBUG));
    let input = Bytes::from_static(b": \xff\r\n\r\ndata: after\n\n");

    for split in 0..=input.len() {
        let chunks = [input.slice(..split), Bytes::new(), input.slice(split..)];
        let stream =
            futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::convert::Infallible>));
        let mut events = SseByteStream::new(stream);
        let event = events
            .next()
            .await
            .expect("data event follows comment")
            .expect("logging must not cause a parse error");
        assert_eq!(event, Sse::default().data("after"), "split={split}");
        assert!(events.next().await.is_none());
    }
}
