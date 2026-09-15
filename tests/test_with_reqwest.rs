use futures_util::StreamExt;
use sse_stream::SseByteStream;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[path = "http/server.rs"]
mod server;

#[tokio::test]
async fn test_axum_with_reqwest() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("info,{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let server_addr = server::start_serve().await?;
    let client = reqwest::Client::new();
    let response = client.get(format!("http://{server_addr}/")).send().await?;
    let mut sse_body = SseByteStream::new(response.bytes_stream());
    let mut receive_count = 0;
    while let Some(Ok(sse)) = sse_body.next().await {
        assert!(sse.data.is_some());
        assert!(sse.event.is_some());
        assert!(sse.id.is_some());
        assert!(sse.retry.is_some());
        receive_count += 1;
    }
    tracing::info!("receive_count: {}", receive_count);
    assert_eq!(receive_count, server::MESSAGE_TOTAL_COUNT);
    Ok(())
}
