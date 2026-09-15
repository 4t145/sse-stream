use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use bytes::Buf;
use futures_util::{
    stream::{FusedStream, MapOk},
    Stream, TryStream, TryStreamExt,
};
use http_body::{Body, Frame};
use http_body_util::{BodyDataStream, StreamBody};

use crate::{Error, Sse};

mod parser;
use parser::Parser;

pin_project_lite::pin_project! {
    /// An SSE decoder over an HTTP body.
    ///
    /// Returns raw event blocks, including metadata-only blocks. After an input
    /// or UTF-8 error it yields that error once and ends. An incomplete final
    /// block is discarded at EOF. See [`SseByteStream`] for byte stream input.
    pub struct SseStream<B: Body> {
        #[pin]
        inner: SseByteStream<BodyDataStream<B>>,
    }
}

pin_project_lite::pin_project! {
    /// An SSE decoder over a stream of byte buffers.
    ///
    /// Input buffers may contain multiple events or fragments of an event.
    /// Returns raw blocks, including metadata-only blocks, without inheriting
    /// ids or retry values across blocks. Unknown fields are ignored; repeated
    /// metadata fields use the last valid value. UTF-8 in recognized values is
    /// validated strictly. Comments are ignored regardless of tracing settings.
    ///
    /// An input or UTF-8 error is returned once, then the stream ends. EOF
    /// discards an incomplete final block. Both endings release parser buffers.
    pub struct SseByteStream<S: TryStream> {
        #[pin]
        stream: S,
        pending_input: Option<S::Ok>,
        parser: Parser,
        finished: bool,
    }
}

impl<S: TryStream> SseByteStream<S> {
    /// Create a decoder from a stream whose successful items implement [`Buf`].
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            pending_input: None,
            parser: Parser::default(),
            finished: false,
        }
    }
}

/// HTTP body adapter used by [`SseStream::from_bytes_stream`].
pub type ByteStreamBody<S, D> = StreamBody<MapOk<S, fn(D) -> Frame<D>>>;

impl<E, S, D> SseStream<ByteStreamBody<S, D>>
where
    S: Stream<Item = Result<D, E>>,
    D: Buf,
{
    /// Decode byte buffers through the HTTP body adapter.
    ///
    /// Prefer [`SseByteStream::new`] for byte streams to avoid HTTP frame adaptation.
    pub fn from_bytes_stream(stream: S) -> Self {
        Self::new(StreamBody::new(
            stream.map_ok(Frame::data as fn(D) -> Frame<D>),
        ))
    }
}

impl<B: Body> SseStream<B> {
    /// Create a decoder from an HTTP body. Trailer frames are ignored.
    pub fn new(body: B) -> Self {
        Self {
            inner: SseByteStream::new(BodyDataStream::new(body)),
        }
    }
}

impl<B: Body> Stream for SseStream<B>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<Sse, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
}

impl<B: Body> FusedStream for SseStream<B>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<S> Stream for SseByteStream<S>
where
    S: TryStream,
    S::Ok: Buf,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    type Item = Result<Sse, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        if *this.finished {
            return Poll::Ready(None);
        }
        let error = loop {
            if let Some(data) = this.pending_input.as_mut() {
                match this.parser.parse_buf(data) {
                    Ok(Some(event)) => {
                        if !data.has_remaining() {
                            *this.pending_input = None;
                        }
                        return Poll::Ready(Some(Ok(event)));
                    }
                    Ok(None) => *this.pending_input = None,
                    Err(error) => break Some(error),
                }
            }
            match ready!(this.stream.as_mut().try_poll_next(cx)) {
                Some(Ok(mut data)) => match this.parser.parse_buf(&mut data) {
                    Ok(Some(event)) => {
                        if data.has_remaining() {
                            *this.pending_input = Some(data);
                        }
                        return Poll::Ready(Some(Ok(event)));
                    }
                    Ok(None) => {}
                    Err(error) => break Some(error),
                },
                Some(Err(error)) => break Some(Error::Body(Box::new(error))),
                None => break None,
            }
        };
        *this.finished = true;
        *this.pending_input = None;
        *this.parser = Parser::default();
        Poll::Ready(error.map(Err))
    }
}

impl<S> FusedStream for SseByteStream<S>
where
    S: TryStream,
    S::Ok: Buf,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    fn is_terminated(&self) -> bool {
        self.finished
    }
}
