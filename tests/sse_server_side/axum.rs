use axum::{
    Router,
    response::sse::{Event, Sse},
    routing::get,
};
use futures_util::{Stream, StreamExt, stream::repeat_with};

use anyhow::Result;
use std::time::Duration;
use tokio::io::{self};

fn router() -> Router {
    Router::new().route("/", get(sse_handler))
}

async fn sse_handler() -> Sse<impl Stream<Item = Result<Event, io::Error>>> {
    tracing::info!("sse connection");
    let mut repeat_count = 0;
    let repeat_limit = 100000;
    let stream = repeat_with(move || {
        repeat_count += 1;
        Ok(Event::default()
            .event("hello")
            .id(repeat_count.to_string())
            .comment("whatever")
            .retry(Duration::from_millis(1000))
            .data(format!("world-{repeat_count}")))
    })
    .take(repeat_limit);
    Sse::new(stream)
}

pub async fn serve(addr: &str) -> io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::debug!("listening on {}", listener.local_addr()?);
    axum::serve(listener, router()).await
}
