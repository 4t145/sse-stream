# Changelog

## 0.3.0 (unreleased)

### Decoding and migration

- `Sse` represents a raw event block. Metadata-only blocks are emitted, fields
  are not inherited across blocks, and connection/reconnection state belongs to
  the caller. This is not a browser `EventSource` implementation.
- Unknown fields are ignored. A field without a colon has an empty value.
  Repeated `event`, `id`, and `retry` fields use the last valid value. An id with
  NULL is ignored. `retry` accepts ASCII digits only; empty, signed, padded, or
  overflowing values are ignored without replacing a preceding valid value.
- UTF-8 validation of recognized field values remains strict. Comments and
  unknown fields are ignored regardless of their encoding or tracing settings.
- An input or UTF-8 error is emitted once, then the decoder ends. Both decoders
  implement `FusedStream` and release parser scratch buffers on error or EOF.
  Incomplete final blocks are discarded. Callers that previously attempted to
  continue after an error must create a new decoder for a new input stream.
- Removed `from_byte_stream`, deprecated since 0.2.4. Use `SseByteStream::new`
  for byte buffers; `SseStream::from_bytes_stream` remains available.
- Removed the obsolete `InvalidLine`, `DuplicatedEventLine`, `DuplicatedIdLine`,
  `DuplicatedRetry`, and `IntParse` error variants. `Error` is now non-exhaustive;
  downstream matches should include a wildcard arm.

### Encoding and keep-alive

- Data is encoded with a `data:` prefix on every logical line, preserving empty
  and trailing lines. CRLF and bare CR in data are normalized to LF.
- Encoding returns `EncodeError::InvalidEvent` if an event type contains CR/LF,
  or `EncodeError::InvalidId` if an id contains CR/LF/NULL. This includes values
  assigned directly to public fields. No bytes are returned for invalid metadata.
  Data may contain line endings.
- Replaced `From<Sse> for Bytes` with `TryFrom<Sse> for Bytes`. Migrate
  `Bytes::from(event)` or `event.into()` to `event.encode()?` or
  `Bytes::try_from(event)?`, and propagate or handle `EncodeError`.
- `KeepAlive::event` now returns `Result<KeepAlive, EncodeError>`; use
  `KeepAlive::new().event(event)?` to validate custom heartbeat events at setup.
- `SseBody` now uses `BodyError` instead of the input stream's error type `E`.
  Input errors must implement `Into<Box<dyn std::error::Error + Send + Sync>>`;
  this supports concrete errors, boxed errors, and infallible streams.
  Match `BodyError::Stream(error)` for upstream failures and
  `BodyError::Encode(error)` for encoding failures. Original error sources are
  preserved in a boxed error. Either error is emitted once, then the body ends;
  keep-alives stop too. `EncodeError` and `BodyError` are non-exhaustive.
- Multiline keep-alive comments prefix every line, preventing embedded text
  from becoming event fields.
- `SseBody::keep_alive` is private. Use `has_keep_alive()` to inspect whether a
  timer is configured and `with_keep_alive()` to configure it.
- `retry_duration` saturates at `u64::MAX` milliseconds instead of wrapping.

### Structure and performance

- Added default `memchr` and `simdutf8` features for line scanning and long-field
  UTF-8 validation, with standard-library fallbacks when disabled.
- HTTP body decoding delegates to the byte-stream decoder, sharing polling and
  error lifecycle handling. Parser states, data buffering, scanning, encoding,
  errors, and keep-alive scheduling have dedicated modules.
- New lines have separate comment, data, and empty-boundary paths. Empty lines
  dispatch directly; complete data lines followed by LF/LF finish the event in
  the same iteration. Fragmented data values append directly to the data buffer.
- After a data event or buffered line completes, each parser scratch buffer
  retains at most 64 KiB. This bounds retained scratch capacity, not the size of
  an in-progress event or the caller's input/output buffers.
- Added feature-matrix CI and targeted coverage for terminal errors, field
  rules, multiline encoding, wake-ups, and keep-alive resets.
- Published crate contents exclude benchmark experiment archives and tooling
  files. Benchmarks and regression tests remain included.
