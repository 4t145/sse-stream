//! Benchmarks adapted from <https://github.com/PizzasBear/sse-rs> (`sse-core/benches/bench.rs`),
//! trimmed to only compare `sse-stream` (this crate) against `sse-core`.
//!
//! Run with: `cargo bench`

use std::{fmt::Write, hint::black_box, time::Duration};

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};
use thiserror::Error;
use tokio_stream::StreamExt;

use sse_core::SseStream as SseCoreStream;

const LARGE_PAYLOAD_SIZE: usize = 40_000;
const MEDIUM_PAYLOAD_SIZE: usize = 4096;
const TCP_CHUNK_SIZE: usize = 1460;
const TINY_CHUNK_SIZE: usize = 10;

fn split_chunks(bytes: &Bytes, chunk_size: usize) -> impl ExactSizeIterator<Item = Bytes> + '_ {
    (0..bytes.len())
        .step_by(chunk_size)
        .map(move |i| bytes.slice(i..bytes.len().min(i + chunk_size)))
}

fn generate_payload(event_count: usize, payload_size: usize) -> Bytes {
    let payload = "word".repeat(payload_size / 4);
    let mut s = String::with_capacity((64 + payload.len()) * event_count);
    for i in 0..event_count {
        write!(
            &mut s,
            "retry:3000\nid: {i}\nevent: message\ndata: payload data: {payload}\n\n"
        )
        .unwrap();
    }
    Bytes::from(s)
}

#[derive(Debug, Clone, Error)]
#[error("{0}")]
struct StrError(String);

fn bench_async_cmp(c: &mut Criterion, group_name: &str, payload: Bytes, chunk_size: usize) {
    let chunks: Vec<Result<_, StrError>> = split_chunks(&payload, chunk_size).map(Ok).collect();

    let mut group = c.benchmark_group(group_name);

    group.throughput(criterion::Throughput::Bytes(payload.len() as _));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    group.bench_function("sse_core", |b| {
        b.to_async(runtime.handle()).iter(|| async {
            let mut sse_stream = SseCoreStream::new(tokio_stream::iter(chunks.iter().cloned()));

            while let Some(Ok(event)) = sse_stream.next().await {
                black_box(event);
            }
        })
    });

    group.bench_function("sse_stream", |b| {
        b.to_async(runtime.handle()).iter(|| async {
            let mut sse_stream = sse_stream::SseStream::from_bytes_stream(tokio_stream::iter(
                chunks.iter().cloned(),
            ));

            while let Some(Ok(event)) = sse_stream.next().await {
                black_box(event);
            }
        })
    });

    group.finish();
}

fn bench_async_parsing_large_events(c: &mut Criterion) {
    let payload = generate_payload(64, LARGE_PAYLOAD_SIZE);

    let group_name = "async_parsing_large_events_tcp_chunks";
    bench_async_cmp(c, group_name, payload, TCP_CHUNK_SIZE);
}

fn bench_async_parsing_small_events(c: &mut Criterion) {
    let payload = Bytes::from("data: {\"t\":\"a\"}\n\n".repeat(100_000));

    let group_name = "async_parsing_small_events";
    bench_async_cmp(c, group_name, payload, TCP_CHUNK_SIZE);
}

fn bench_async_parsing_keepalives(c: &mut Criterion) {
    let payload = Bytes::from(": keepalive\n\n".repeat(200_000));

    let group_name = "async_parsing_keepalives";
    bench_async_cmp(c, group_name, payload, TCP_CHUNK_SIZE);
}

fn bench_async_parsing_medium_events_tiny_chunks(c: &mut Criterion) {
    let payload = generate_payload(256, MEDIUM_PAYLOAD_SIZE);

    let group_name = "async_parsing_medium_events_tiny_chunks";
    bench_async_cmp(c, group_name, payload, TINY_CHUNK_SIZE);
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(200)
        .measurement_time(Duration::from_secs(15));
    targets =
        bench_async_parsing_large_events,
        bench_async_parsing_small_events,
        bench_async_parsing_keepalives,
        bench_async_parsing_medium_events_tiny_chunks,
}
criterion_main!(benches);
