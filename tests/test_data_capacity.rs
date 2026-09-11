use std::convert::Infallible;

use bytes::Bytes;
use futures_util::StreamExt;
use sse_stream::{Sse, SseByteStream};

#[tokio::test]
async fn small_events_after_large_events_have_compact_owned_data() {
    let large = "中文🙂é".repeat(4000);
    let mut input = format!("data: {large}\n\n");
    input.push_str("id: control\n\ndata:\n\n");
    for _ in 0..32 {
        input.push_str("data: x\ndata: y\n\n");
    }
    input.push_str(&format!("data: {large}\r\n\r\ndata: tail\n\n"));

    for width in [1, 4, 255, 1460, input.len()] {
        let chunks = input
            .as_bytes()
            .chunks(width)
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)));
        // Keep all outputs alive to exercise a consumer that queues events.
        let events: Vec<Sse> = SseByteStream::new(futures_util::stream::iter(chunks))
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(events.len(), 37);
        assert_eq!(events[0].data.as_deref(), Some(large.as_str()));
        assert_eq!(events[1], Sse::default().id("control"));
        assert_eq!(events[2].data.as_deref(), Some(""));
        assert_eq!(events[35].data.as_deref(), Some(large.as_str()));
        assert_eq!(events[36].data.as_deref(), Some("tail"));
        for event in &events[3..35] {
            assert_eq!(event.data.as_deref(), Some("x\ny"));
        }
        for index in (2..35).chain(std::iter::once(36)) {
            let data = events[index].data.as_ref().unwrap();
            assert!(
                data.capacity() <= 64,
                "small output retained a large allocation: width={width}, index={index}, capacity={}",
                data.capacity()
            );
        }
    }
}

#[tokio::test]
async fn varied_sizes_keep_complete_payloads() {
    let sizes = [32_768, 4096, 8191, 8192, 8193, 512, 16_384, 80, 40_000];
    let values: Vec<_> = sizes.iter().map(|&size| "x".repeat(size)).collect();
    let input: String = values
        .iter()
        .map(|value| format!("data: {value}\ndata: tail\n\n"))
        .collect();
    for width in [10, 1460, input.len()] {
        let chunks = input.as_bytes().chunks(width).map(Ok::<_, Infallible>);
        let mut events = SseByteStream::new(futures_util::stream::iter(chunks));
        for value in &values {
            let event = events.next().await.unwrap().unwrap();
            assert_eq!(
                event.data.as_deref(),
                Some(format!("{value}\ntail").as_str())
            );
        }
        assert!(events.next().await.is_none());
    }
}
