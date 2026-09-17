use std::{
    pin::Pin,
    task::{Context, Poll},
};

use crate::{BodyError, Sse};
use bytes::Bytes;
use futures_util::Stream;
use http_body::{Body, Frame};
mod keep_alive;
use keep_alive::KeepAliveStream;
pub use keep_alive::*;

#[cfg(test)]
mod tests;
pin_project_lite::pin_project! {
    /// Encode SSE events as HTTP data frames, optionally sending keep-alives.
    ///
    /// Input and encoding errors are returned once as [`BodyError`], then the
    /// body ends without sending further events or keep-alives.
    ///
    /// Data may contain line endings; they are normalized to LF.
    pub struct SseBody<S, T = NeverTimer> {
        #[pin]
        pub event_stream: S,
        #[pin]
        keep_alive: Option<KeepAliveStream<T>>,
        finished: bool,
    }
}

impl<S, T> SseBody<S, T> {
    /// Whether a keep-alive timer is configured.
    pub fn has_keep_alive(&self) -> bool {
        self.keep_alive.is_some()
    }
}

impl<S, E> SseBody<S, NeverTimer>
where
    S: Stream<Item = Result<Sse, E>>,
{
    pub fn new(stream: S) -> Self {
        Self {
            event_stream: stream,
            keep_alive: None,
            finished: false,
        }
    }
}

impl<S, E, T> SseBody<S, T>
where
    S: Stream<Item = Result<Sse, E>>,
    T: Timer,
{
    pub fn new_keep_alive(stream: S, keep_alive: KeepAlive) -> Self {
        Self {
            event_stream: stream,
            keep_alive: Some(KeepAliveStream::new(keep_alive)),
            finished: false,
        }
    }

    pub fn with_keep_alive<T2: Timer>(self, keep_alive: KeepAlive) -> SseBody<S, T2> {
        SseBody {
            event_stream: self.event_stream,
            keep_alive: Some(KeepAliveStream::new(keep_alive)),
            finished: self.finished,
        }
    }
}

impl<S, E, T> Body for SseBody<S, T>
where
    S: Stream<Item = Result<Sse, E>>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
    T: Timer,
{
    type Data = Bytes;
    type Error = BodyError;

    /// # Errors
    ///
    /// Returns [`BodyError::Stream`] for an input error or [`BodyError::Encode`]
    /// for invalid metadata. Either error ends the body.
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        if *this.finished {
            return Poll::Ready(None);
        }

        match this.event_stream.poll_next(cx) {
            Poll::Pending => {
                if let Some(keep_alive) = this.keep_alive.as_pin_mut() {
                    keep_alive.poll_event(cx).map(|e| Some(Ok(Frame::data(e))))
                } else {
                    Poll::Pending
                }
            }
            Poll::Ready(Some(Ok(event))) => {
                let bytes = match event.encode() {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        *this.finished = true;
                        return Poll::Ready(Some(Err(BodyError::Encode(error))));
                    }
                };
                if let Some(keep_alive) = this.keep_alive.as_pin_mut() {
                    keep_alive.reset();
                }
                Poll::Ready(Some(Ok(Frame::data(bytes))))
            }
            Poll::Ready(Some(Err(error))) => {
                *this.finished = true;
                Poll::Ready(Some(Err(BodyError::Stream(error.into()))))
            }
            Poll::Ready(None) => {
                *this.finished = true;
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.finished
    }
}
