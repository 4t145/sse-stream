//! End-to-end stream benchmarks comparing this crate with `sse-core` across
//! event shapes and input fragmentation patterns.
//!
//! All comparison workloads produce the same number of logical message events
//! in both implementations. `retry` is deliberately excluded because
//! `sse-core` yields it as a standalone event while `sse-stream` attaches it to
//! the next [`sse_stream::Sse`], which makes a throughput ratio misleading.
//!
//! Run the full suite with `cargo bench`, or use `cargo bench -- --quick` while
//! iterating locally.

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use sse_core::SseStream as SseCoreStream;
use std::{hint::black_box, time::Duration};
use thiserror::Error;
use tokio_stream::StreamExt;
mod scenarios;

#[derive(Debug, Clone, Error)]
#[error("{0}")]
struct StrError(String);

fn bench_async_cmp(
    c: &mut Criterion,
    group_name: &str,
    chunks: Vec<Bytes>,
    expected_events: usize,
) {
    let payload_len: usize = chunks.iter().map(Bytes::len).sum();
    let chunks: Vec<Result<_, StrError>> = chunks.into_iter().map(Ok).collect();

    let mut group = c.benchmark_group(group_name);
    group.throughput(criterion::Throughput::Bytes(payload_len as u64));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();

    group.bench_function("sse_core", |b| {
        b.to_async(runtime.handle()).iter(|| async {
            let mut stream = SseCoreStream::new(tokio_stream::iter(chunks.iter().cloned()));
            let mut event_count = 0;

            while let Some(event) = stream.next().await {
                black_box(event.expect("sse-core parse error"));
                event_count += 1;
            }

            assert_eq!(event_count, expected_events);
        })
    });

    group.bench_function("sse_stream", |b| {
        b.to_async(runtime.handle()).iter(|| async {
            let mut stream =
                sse_stream::SseByteStream::new(tokio_stream::iter(chunks.iter().cloned()));
            let mut event_count = 0;

            while let Some(event) = stream.next().await {
                black_box(event.expect("sse-stream parse error"));
                event_count += 1;
            }

            assert_eq!(event_count, expected_events);
        })
    });

    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    for case in scenarios::decode_cases() {
        bench_async_cmp(c, &case.name, case.chunks, case.events);
    }
}

fn bench_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode");
    for case in scenarios::encode_cases() {
        let event = sse_stream::Sse {
            event: case.event,
            data: Some(case.data),
            id: case.id,
            retry: case.retry,
        };
        let wire = Bytes::from(event.clone());
        group.throughput(criterion::Throughput::Bytes(wire.len() as u64));
        group.bench_function(case.name, |b| {
            b.iter_batched(
                || event.clone(),
                |event| black_box(Bytes::from(event)),
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(10));
    targets = bench_decode, bench_encode,
}
criterion_main!(benches);
