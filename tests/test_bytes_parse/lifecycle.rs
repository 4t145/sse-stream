use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::Poll,
};

use futures_util::{
    stream::{self, FusedStream},
    TryStreamExt,
};

use super::*;

#[tokio::test]
async fn parse_errors_end_both_decoders_for_every_chunk_width() {
    let input = b"data: before\ndata: \xff\n\ndata: after\n\n";
    for width in 1..=input.len() {
        for body in [false, true] {
            let chunks = stream::iter(input.chunks(width).map(Ok::<_, Infallible>));
            let mut events: std::pin::Pin<
                Box<dyn FusedStream<Item = Result<Sse, sse_stream::Error>>>,
            > = if body {
                Box::pin(SseStream::new(StreamBody::new(chunks.map_ok(Frame::data))))
            } else {
                Box::pin(SseByteStream::new(chunks))
            };
            assert!(matches!(
                events.next().await,
                Some(Err(sse_stream::Error::Utf8Parse(_)))
            ));
            assert!(events.is_terminated());
            assert!(events.next().await.is_none(), "width={width}, body={body}");
            assert!(events.next().await.is_none());
        }
    }
}

#[tokio::test]
async fn invalid_first_data_line_never_creates_an_empty_event() {
    for chunks in [
        vec![&b"data: \xff\n"[..], b"\n"],
        vec![&b"data: \xff"[..], b"\n", b"\n"],
    ] {
        let mut events =
            SseByteStream::new(stream::iter(chunks.into_iter().map(Ok::<_, Infallible>)));
        assert!(matches!(
            events.next().await,
            Some(Err(sse_stream::Error::Utf8Parse(_)))
        ));
        assert!(events.next().await.is_none());
    }
}

#[tokio::test]
async fn completed_events_precede_error_and_input_is_never_polled_again() {
    for body in [false, true] {
        let polls = Arc::new(AtomicUsize::new(0));
        let count = polls.clone();
        let input = stream::poll_fn(move |_| {
            let result = match count.fetch_add(1, Ordering::Relaxed) {
                0 => Ok(Bytes::from_static(b"data: first\n\ndata: incomplete")),
                1 => Err(std::io::Error::other("transport failed")),
                _ => panic!("input polled after terminal error"),
            };
            Poll::Ready(Some(result))
        });
        let mut events: std::pin::Pin<Box<dyn FusedStream<Item = Result<Sse, sse_stream::Error>>>> =
            if body {
                Box::pin(SseStream::new(StreamBody::new(input.map_ok(Frame::data))))
            } else {
                Box::pin(SseByteStream::new(input))
            };
        assert_eq!(
            events
                .next()
                .await
                .expect("first event")
                .expect("valid event"),
            data_only("first")
        );
        let error = events
            .next()
            .await
            .expect("transport error")
            .expect_err("input failed");
        assert_eq!(
            std::error::Error::source(&error)
                .expect("original error")
                .to_string(),
            "transport failed"
        );
        assert!(events.next().await.is_none());
        assert_eq!(polls.load(Ordering::Relaxed), 2);
    }
}

#[tokio::test]
async fn partial_utf8_survives_pending_and_wakeup() {
    for body in [false, true] {
        let mut chunks = [b"data: \xe4".as_slice(), b"\xbd\xa0\r", b"\n\r", b"\n"].into_iter();
        let mut pending = false;
        let input = stream::poll_fn(move |cx| {
            pending = !pending;
            if pending {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(chunks.next().map(Ok::<_, Infallible>))
            }
        });
        let events: Vec<_> = if body {
            SseStream::new(StreamBody::new(input.map_ok(Frame::data)))
                .try_collect()
                .await
        } else {
            SseByteStream::new(input).try_collect().await
        }
        .expect("valid fragmented stream");
        assert_eq!(events, vec![data_only("你")]);
    }
}

#[tokio::test]
async fn eof_discards_partial_data_and_fuses_non_fused_input() {
    let mut step = 0;
    let input = stream::poll_fn(move |_| {
        step += 1;
        Poll::Ready(match step {
            1 => Some(Ok::<_, Infallible>(b"data: partial".as_slice())),
            2 => None,
            _ => panic!("input polled after EOF"),
        })
    });
    let mut events = SseByteStream::new(input);
    assert!(!events.is_terminated());
    assert!(events.next().await.is_none());
    assert!(events.is_terminated());
    assert!(events.next().await.is_none());
}
