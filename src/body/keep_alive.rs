use crate::{EncodeError, Sse};
use bytes::Bytes;
use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};

const DEFAULT_INTERVAL: Duration = Duration::from_secs(15);

/// Configure the interval between keep-alive messages, the content
/// of each message, and the associated stream.
#[derive(Debug, Clone)]
#[must_use]
pub struct KeepAlive {
    event: Bytes,
    max_interval: Duration,
}

impl KeepAlive {
    /// Create a new `KeepAlive`.
    pub fn new() -> Self {
        Self {
            event: Bytes::from_static(b":\n\n"),
            max_interval: DEFAULT_INTERVAL,
        }
    }

    /// Customize the interval between keep-alive messages.
    ///
    /// Default is 15 seconds.
    pub fn interval(mut self, time: Duration) -> Self {
        self.max_interval = time;
        self
    }

    /// Customize the event of the keep-alive message.
    ///
    /// Default is an empty comment.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] if the event type or id contains CR/LF, or the
    /// id contains NULL. Multiline data is allowed.
    pub fn event(mut self, event: Sse) -> Result<Self, EncodeError> {
        self.event = event.encode()?;
        Ok(self)
    }

    /// Customize the keep-alive comment. Each logical line is encoded as a comment.
    pub fn comment(mut self, comment: &str) -> Self {
        self.event = crate::event::encode::encode_comment(comment);
        self
    }
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime-independent keep-alive timer.
///
/// Implementations must register the current waker when pending and arrange a
/// wake-up at the deadline. `reset` must rearm even a previously completed timer.
pub trait Timer: Future<Output = ()> {
    /// Set the next deadline.
    fn reset(self: Pin<&mut Self>, instant: std::time::Instant);
    /// Create a timer that completes after the given duration.
    fn from_duration(duration: Duration) -> Self;
}

pub struct NeverTimer;

impl Future for NeverTimer {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Timer for NeverTimer {
    fn from_duration(_: Duration) -> Self {
        Self
    }

    fn reset(self: Pin<&mut Self>, _: std::time::Instant) {
        // No-op
    }
}

pin_project_lite::pin_project! {
    #[derive(Debug)]
    pub(super) struct KeepAliveStream<S> {
        keep_alive: KeepAlive,
        #[pin]
        alive_timer: S,
    }
}

impl<S> KeepAliveStream<S>
where
    S: Timer,
{
    pub(super) fn new(keep_alive: KeepAlive) -> Self {
        Self {
            alive_timer: S::from_duration(keep_alive.max_interval),
            keep_alive,
        }
    }

    pub(super) fn reset(self: Pin<&mut Self>) {
        let this = self.project();
        this.alive_timer
            .reset(std::time::Instant::now() + this.keep_alive.max_interval);
    }

    pub(super) fn poll_event(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Bytes> {
        let this = self.as_mut().project();

        ready!(this.alive_timer.poll(cx));

        let event = this.keep_alive.event.clone();

        self.reset();

        Poll::Ready(event)
    }
}
