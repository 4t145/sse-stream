use axum::{
    response::sse::{Event, Sse},
    routing::get,
    Router,
};
use futures_util::{stream::repeat_with, Stream, StreamExt};

use anyhow::Result;
use std::{net::SocketAddr, time::Duration};
use tokio::io::{self};

const TEST_ADDR_ENV: &str = "SSE_STREAM_TEST_ADDR";

fn router() -> Router {
    Router::new().route("/", get(sse_handler))
}

pub const MESSAGE_TOTAL_COUNT: usize = 100000;
async fn sse_handler() -> Sse<impl Stream<Item = Result<Event, io::Error>>> {
    tracing::info!("sse connection");
    let mut repeat_count = 0;
    let stream = repeat_with(move || {
        repeat_count += 1;
        Ok(Event::default()
            .event("hello")
            .id(repeat_count.to_string())
            .comment("whatever")
            .retry(Duration::from_millis(1000))
            .data(format!("world-{repeat_count}")))
    })
    .take(MESSAGE_TOTAL_COUNT);
    Sse::new(stream)
}

pub async fn start_serve() -> io::Result<SocketAddr> {
    let addr = std::env::var(TEST_ADDR_ENV).unwrap_or_else(|_| "127.0.0.1:0".to_owned());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;

    tracing::debug!(%local_addr, %addr, "listening for integration test");
    tokio::spawn(async move { axum::serve(listener, router()).await });
    Ok(local_addr)
}
