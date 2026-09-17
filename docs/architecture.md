# Architecture

## Module responsibilities

| Module | Responsibility |
| --- | --- |
| `src/stream.rs` | Poll byte input, adapt HTTP bodies, and end decoders on EOF or error |
| `src/stream/parser.rs` | Consume lines and dispatch raw event blocks |
| `src/stream/parser/data.rs` | Accumulate data fragments and track validated UTF-8 |
| `src/stream/parser/scan.rs` | Find line delimiters and validate UTF-8 |
| `src/event.rs` | Represent an event and provide field builders |
| `src/event/encode.rs` | Validate metadata and encode event or comment lines |
| `src/error.rs` | Define decoding, encoding, and body errors |
| `src/body.rs` | Encode an event stream into HTTP data frames |
| `src/body/keep_alive.rs` | Configure heartbeat content and manage its timer |

## Input and parser state

`SseStream` adapts HTTP data frames into `SseByteStream`. The byte decoder retains
unconsumed input when a buffer contains several events. `Parser::parse_buf`
consumes every segment exposed by `Buf::chunk`, including segmented buffers.

The parser tracks four line states:

- `Start`: the next byte begins a new line.
- `Buffered`: retain a partial field, prefix, or comment needed for tracing.
- `Data`: the complete `data:` prefix is known; append fragments directly to
  the event data buffer. The state remembers whether an optional leading space
  still needs to be consumed.
- `Comment`: discard fragments until the comment's line delimiter arrives.

A CR at the end of a chunk records that the next leading LF belongs to the same
delimiter. A BOM is stripped only at the start of the first line.

Only `Start` may classify input as a new comment, data field, or empty line.
A buffered field's continuation can itself begin with `:` or `data:` without
starting a new field. An empty line dispatches any collected fields, including
metadata-only blocks. Field values do not carry over to the next block.

Unknown field names are ignored by default. The opt-in `strict-fields` feature
returns `Error::UnknownField` for them instead, without UTF-8 validation of the
unknown name or value. This includes colonless unknown fields, but not comments,
known colonless fields, invalid retries, or NULL-containing ids. The error uses
the same terminal lifecycle as other parser errors. Cargo feature unification
makes this a crate-wide policy for both decoder adapters, not per-instance configuration.

## Scanning and event completion

Data lines use a dedicated delimiter scan. With `memchr`, long slices use
`memchr2` directly; general lines scan a short scalar prefix first. Without the
feature, both paths use scalar scanning.

Complete data followed by LF/LF finishes the event immediately. Other empty
new lines have a separate dispatch branch before general field parsing. This
keeps event completion from entering `parse_line`, whose larger return path
adds overhead even for an empty input. Preserve both paths when reorganizing
the parser, and measure metadata-after-data, CRLF, and fragmented input.

## Data ownership and validation

`DataBuffer::validated_len` separates completed UTF-8 data lines from an
unfinished suffix. Each completed line is validated before it becomes visible
to event dispatch. The optional `simdutf8` feature accelerates long valid fields;
short fields and validation failures use the standard library, preserving its
UTF-8 error details.

Dispatch removes the final data separator and transfers the buffer into an
owned `String`. Small results in a disproportionately large buffer are copied
into compact storage instead. Each scratch buffer retains at most 64 KiB after
an event or buffered line completes. In-progress events and caller-owned input
or output buffers are not subject to that retained-capacity bound.

EOF discards unfinished events. An input or parsing error is emitted once,
then the decoder ends and releases scratch buffers. Both decoders implement
`FusedStream`.

## Encoding and keep-alives

Encoding validates event types and ids before returning any bytes. Each data
line receives its own prefix; empty and trailing lines are preserved, and CRLF
or bare CR are normalized to LF. The encoder calculates output capacity before
writing the block.

`SseBody` polls its input first. When input is pending, a configured keep-alive
timer can yield a heartbeat. Sending an event or heartbeat resets the timer;
EOF or an error ends the body. Timer implementations must register wake-ups and
support resetting a timer that has already completed.

Custom keep-alive events are validated when configured. Every line of a
keep-alive comment receives a comment prefix.
