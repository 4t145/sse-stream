# Changelog

## 0.3.0 (unreleased)

### Breaking API changes

- **Encoding:** `TryFrom<Sse> for Bytes` replaces `From<Sse> for Bytes`.
  Replace `Bytes::from(event)` or `event.into()` with `event.encode()?` or
  `Bytes::try_from(event)?`.
- **Keep-alive events:** `KeepAlive::event` returns `Result<KeepAlive, EncodeError>`.
  Use `KeepAlive::new().event(event)?`.
- **Body errors:** `SseBody::Error` is now `BodyError`, not the input error `E`.
  Match `BodyError::Stream` for boxed upstream errors and `BodyError::Encode`
  for encoding errors; upstream error sources are preserved.
- **Input error bounds:** `SseBody` requires
  `E: Into<Box<dyn std::error::Error + Send + Sync>>`.
- **Error matching:** Removed `InvalidLine`, `DuplicatedEventLine`,
  `DuplicatedIdLine`, `DuplicatedRetry`, and `IntParse`.
  `Error`, `EncodeError`, and `BodyError` are non-exhaustive; add wildcard arms.
- **Decoder constructor:** Removed `from_byte_stream`, deprecated since 0.2.4.
  Use `SseByteStream::new` or the retained `SseStream::from_bytes_stream`.
- **Decoder auto traits:** `SseStream<B>` now also requires `B::Data: Send`
  or `Sync` to implement the corresponding trait, since it retains input buffers.
  Standard `Bytes` buffers are unaffected.
- **Body construction:** Struct literals are no longer supported.
  Use `SseBody::new(stream)` or `SseBody::new_keep_alive(stream, config)`.
- **Keep-alive access:** `SseBody::keep_alive` is private.
  Use `has_keep_alive()` to inspect it and `with_keep_alive()` to configure it.
  Directly clearing the configuration is no longer supported.

### Behavior changes

- **Terminal errors:** Decoders and `SseBody` emit an error once, then end.
  Keep-alives stop too. To resume decoding, create a decoder for a new input stream.
- **Unknown fields:** Ignored by default instead of rejected; colonless fields have empty values.
  The opt-in `strict-fields` feature rejects unknown names with the always-public
  `Error::UnknownField` unit variant, emitted once before termination. Cargo
  feature unification applies this policy to all users of the same resolved crate.
- **Repeated metadata:** `event`, `id`, and `retry` use the last valid value.
- **Retry parsing:** Only ASCII digits fitting in `u64` are accepted.
  Empty, signed, padded, or overflowing values are ignored without replacing a valid value.
- **Ignored text:** Comments are ignored regardless of encoding or tracing settings.
  Unknown fields are also ignored regardless of encoding unless `strict-fields`
  is enabled; strict mode rejects unknown names without validating their UTF-8.
- **Metadata validation:** Encoding rejects CR/LF in event types and CR/LF/NULL
  in ids, including direct field assignments. No bytes are returned on failure.
- **Multiline data:** Every line receives a `data:` prefix. Empty and trailing
  lines are preserved; CRLF and bare CR are normalized to LF.
- **Keep-alive comments:** Every line receives a comment prefix, preventing field injection.
- **Retry duration:** `retry_duration` saturates at `u64::MAX` milliseconds instead of wrapping.

### Clarified existing behavior

- `Sse` represents raw blocks, including metadata-only blocks. Fields are not
  inherited; connection and reconnection state belongs to the caller.
- Recognized field values require valid UTF-8; ids containing NULL are ignored.
- Incomplete final blocks are discarded at EOF.

### Structure and performance

- Both decoders implement `FusedStream` and release parser buffers on error or EOF.
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
