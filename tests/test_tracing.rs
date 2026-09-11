#![cfg(feature = "tracing")]

// Isolate debug tracing from parallel parser tests with no subscriber.
use bytes::Bytes;
use futures_util::StreamExt;
use sse_stream::SseByteStream;

#[tokio::test]
async fn test_fragmented_comment_utf8_validation_with_debug_tracing() {
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
        let error = events
            .next()
            .await
            .expect("invalid comment produces an error with debug tracing")
            .expect_err(&format!("comment contains invalid UTF-8, split={split}"));
        assert!(matches!(error, sse_stream::Error::Utf8Parse(_)));
    }
}
