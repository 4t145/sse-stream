use std::{
    cell::RefCell,
    convert::Infallible,
    error::Error as _,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use bytes::Bytes;
use futures_util::{
    stream,
    task::{waker, ArcWake},
    StreamExt,
};
use http_body::Body;
use tokio::sync::mpsc;

use super::{KeepAlive, SseBody, Timer};
use crate::{BodyError, EncodeError, Sse};

// Each test controls one timer on its own thread without sleeping or changing
// the Tokio runtime features used by the production benchmarks.
#[derive(Default)]
struct Clock {
    ready: bool,
    deadline: Option<Instant>,
    resets: usize,
    waker: Option<Waker>,
}

thread_local! {
    static CLOCK: RefCell<Clock> = RefCell::default();
}

struct ManualTimer;

impl Future for ManualTimer {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        CLOCK.with_borrow_mut(|clock| {
            if clock.ready {
                Poll::Ready(())
            } else {
                clock.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
    }
}

impl Timer for ManualTimer {
    fn reset(self: Pin<&mut Self>, deadline: Instant) {
        CLOCK.with_borrow_mut(|clock| {
            clock.ready = false;
            clock.deadline = Some(deadline);
            clock.resets += 1;
        });
    }
    fn from_duration(duration: Duration) -> Self {
        CLOCK.set(Clock {
            deadline: Some(Instant::now() + duration),
            ready: duration.is_zero(),
            ..Clock::default()
        });
        Self
    }
}

fn expire_timer() {
    let waker = CLOCK.with_borrow_mut(|clock| {
        clock.ready = true;
        clock.waker.take()
    });
    waker.expect("pending timer registered a waker").wake();
}

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl ArcWake for WakeCount {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.fetch_add(1, Ordering::Relaxed);
    }
}

fn poll_data<B>(mut body: Pin<&mut B>, cx: &mut Context<'_>) -> Poll<Option<Bytes>>
where
    B: Body<Data = Bytes, Error = BodyError>,
{
    body.as_mut().poll_frame(cx).map(|frame| {
        frame.map(|frame| {
            frame
                .expect("valid event from infallible stream")
                .into_data()
                .expect("data frame")
        })
    })
}

#[test]
fn keep_alive_wakes_resets_and_stops_when_input_ends() {
    let (sender, mut receiver) = mpsc::unbounded_channel::<Result<Sse, Infallible>>();
    let input = stream::poll_fn(move |cx| receiver.poll_recv(cx));
    let interval = Duration::from_secs(10);
    let mut body = Box::pin(
        SseBody::new(input).with_keep_alive::<ManualTimer>(KeepAlive::new().interval(interval)),
    );
    assert!(body.has_keep_alive());
    let count = Arc::new(WakeCount::default());
    let wake = waker(count.clone());
    let mut cx = Context::from_waker(&wake);
    assert!(poll_data(body.as_mut(), &mut cx).is_pending());
    expire_timer();
    assert!(count.0.load(Ordering::Relaxed) > 0);
    assert_eq!(
        poll_data(body.as_mut(), &mut cx),
        Poll::Ready(Some(Bytes::from_static(b":\n\n")))
    );
    assert_eq!(CLOCK.with_borrow(|clock| clock.resets), 1);
    assert!(poll_data(body.as_mut(), &mut cx).is_pending());

    // Real events take priority even when a keep-alive is also ready.
    expire_timer();
    sender
        .send(Ok(Sse::default().data("event")))
        .expect("receiver alive");
    let before_event = Instant::now();
    assert_eq!(
        poll_data(body.as_mut(), &mut cx),
        Poll::Ready(Some(Bytes::from_static(b"data: event\n\n")))
    );
    let after_event = Instant::now();
    CLOCK.with_borrow(|clock| {
        assert_eq!(clock.resets, 2);
        let deadline = clock.deadline.expect("rearmed timer");
        assert!(deadline >= before_event + interval && deadline <= after_event + interval);
    });
    assert!(
        poll_data(body.as_mut(), &mut cx).is_pending(),
        "event must postpone keep-alive"
    );
    expire_timer();
    assert_eq!(
        poll_data(body.as_mut(), &mut cx),
        Poll::Ready(Some(Bytes::from_static(b":\n\n")))
    );
    assert!(poll_data(body.as_mut(), &mut cx).is_pending());
    expire_timer();
    drop(sender);
    assert_eq!(poll_data(body.as_mut(), &mut cx), Poll::Ready(None));
    assert!(body.is_end_stream());
    assert_eq!(poll_data(body.as_mut(), &mut cx), Poll::Ready(None));
}

#[test]
fn body_errors_stop_input_and_keep_alives() {
    for item in [
        Ok(Sse::default().event("bad\nevent")),
        Ok(Sse::default().id("bad\0id")),
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "source closed",
        )),
    ] {
        let expected_encoding_error = match &item {
            Ok(event) if event.event.is_some() => Some(EncodeError::InvalidEvent),
            Ok(_) => Some(EncodeError::InvalidId),
            Err(_) => None,
        };
        let input = stream::iter([item]).chain(stream::poll_fn(|_| {
            panic!("a failed body must not poll its input again")
        }));
        let mut body = Box::pin(SseBody::<_, ManualTimer>::new_keep_alive(
            input,
            KeepAlive::new().interval(Duration::ZERO),
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(!body.is_end_stream());
        let Poll::Ready(Some(Err(error))) = body.as_mut().poll_frame(&mut cx) else {
            panic!("expected a body error before the ready keep-alive");
        };
        match (&error, expected_encoding_error) {
            (BodyError::Encode(actual), Some(expected)) => {
                assert_eq!(*actual, expected);
                assert_eq!(
                    error
                        .source()
                        .and_then(|source| source.downcast_ref::<EncodeError>()),
                    Some(&expected)
                );
            }
            (BodyError::Stream(_), None) => {
                let source = error
                    .source()
                    .and_then(|source| source.downcast_ref::<std::io::Error>())
                    .expect("original upstream error");
                assert_eq!(source.kind(), std::io::ErrorKind::BrokenPipe);
                assert_eq!(source.to_string(), "source closed");
            }
            _ => panic!("wrong body error: {error:?}"),
        }
        assert!(body.is_end_stream());
        for _ in 0..2 {
            assert!(matches!(
                body.as_mut().poll_frame(&mut cx),
                Poll::Ready(None)
            ));
        }
        assert_eq!(CLOCK.with_borrow(|clock| clock.resets), 0);
    }
}

#[test]
fn custom_keep_alive_event_uses_validated_encoding() {
    let input = stream::pending::<Result<Sse, Infallible>>();
    let keep_alive = KeepAlive::new()
        .interval(Duration::ZERO)
        .event(Sse::default().event("ping").id("1").data("a\r\nb\n"))
        .expect("valid keep-alive metadata");
    let mut body = Box::pin(SseBody::<_, ManualTimer>::new_keep_alive(input, keep_alive));
    let mut cx = Context::from_waker(Waker::noop());
    assert_eq!(
        poll_data(body.as_mut(), &mut cx),
        Poll::Ready(Some(Bytes::from_static(
            b"event: ping\ndata: a\ndata: b\ndata: \nid: 1\n\n"
        )))
    );
}

#[tokio::test]
async fn multiline_keep_alive_comments_cannot_inject_events() {
    let input = stream::pending::<Result<Sse, std::convert::Infallible>>();
    let mut body = Box::pin(SseBody::<_, ManualTimer>::new_keep_alive(
        input,
        KeepAlive::new()
            .interval(Duration::ZERO)
            .comment("hello\r\ndata: injected\n\n"),
    ));
    let frame = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx))
        .await
        .expect("keep-alive frame")
        .expect("infallible body")
        .into_data()
        .expect("data frame");
    assert_eq!(frame.as_ref(), b": hello\n: data: injected\n: \n: \n\n");
    let mut events = crate::SseStream::new(http_body_util::Full::new(frame));
    assert!(futures_util::StreamExt::next(&mut events).await.is_none());
}

#[test]
fn body_without_timer_reports_keep_alive_disabled() {
    let body = SseBody::new(stream::empty::<Result<Sse, std::convert::Infallible>>());
    assert!(!body.has_keep_alive());
}
