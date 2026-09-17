# Benchmarking

## Workloads

The Criterion suite in `benches/bench.rs` uses fixtures from
`benches/scenarios.rs`:

- 28 decoding workloads cover small messages, metadata order, multiline data,
  LF/CRLF, comments, ASCII and Unicode payloads, mixed event sizes, and input
  fragments from 4 bytes to whole streams.
- 5 encoding workloads cover small JSON, metadata, large ASCII and Unicode
  data, and an empty event with retry metadata.

Decoding compares `SseByteStream` with `sse-core` on the same input chunks.
Each workload asserts its expected event count. Retry is excluded from these
comparisons because `sse-core` emits it separately while this crate attaches it
to the next event block. Encoding has no `sse-core` comparison.

Decoding timings include input polling, buffer clone/drop, parsing, and dropping
the output events. Encoding timings exclude cloning the input event. These are
in-memory measurements; they do not measure network latency or application JSON
deserialization.

## Run the suite

```sh
cargo bench --bench bench
```

For a quick check during development:

```sh
cargo bench --bench bench -- --quick
```

Filter to this crate's parser or an individual workload:

```sh
cargo bench --bench bench -- '/sse_stream$'
cargo bench --bench bench -- '^json_crlf_tcp/sse_stream$'
cargo bench --bench bench -- '^encode/'
```

To exercise the scalar implementation:

```sh
cargo bench --no-default-features --bench bench -- '/sse_stream$'
```

Criterion writes reports under `target/criterion`. Use `CRITERION_HOME` to put
an experiment's results in another directory outside the source tree.

## Compare revisions

Build both versions before timing, then run the saved benchmark executables
serially. Use identical fixtures, compiler settings, features, and input types.
If a public API changes, adapt the harness without changing the workload or
including compatibility work inside only one version's timed loop.

A short measurement configuration is:

```sh
cargo bench --bench bench -- '/sse_stream$' \
  --sample-size 40 --warm-up-time 0.2 --measurement-time 1 --noplot
```

For a release comparison, measure each workload in several adjacent rounds,
alternating version order. On Linux, `taskset -c <cpu>` can keep both processes
on the same logical CPU. Longer measurements may be needed on a busy machine.

Record the revisions, source and fixture hashes, compiler, features, commands,
CPU selection, per-round means, and confidence intervals. Keep the original
samples when investigating an outlier. A range across rounds is not a confidence
interval, and a difference measured on one host does not establish a portable
speedup.

After a parser change, check all decoding workloads. Include ordinary small
messages and comments as controls alongside metadata-after-data, CRLF, and
extremely fragmented input; a shortcut can help one event shape while changing
the generated code for another.
