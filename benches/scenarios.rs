use bytes::Bytes;
use std::fmt::Write;

pub struct DecodeCase {
    pub name: String,
    pub chunks: Vec<Bytes>,
    pub events: usize,
}

fn add_case(cases: &mut Vec<DecodeCase>, name: &str, chunks: Vec<Bytes>, events: usize) {
    cases.push(DecodeCase {
        name: name.to_owned(),
        chunks,
        events,
    });
}

const LARGE_PAYLOAD_SIZE: usize = 40_000;
const MEDIUM_PAYLOAD_SIZE: usize = 4096;
const TCP_CHUNK_SIZE: usize = 1460;
const TINY_CHUNK_SIZE: usize = 10;
const PREFIX_SPLIT_CHUNK_SIZE: usize = 4;

const LARGE_EVENT_COUNT: usize = 64;
const MEDIUM_EVENT_COUNT: usize = 256;
const SMALL_EVENT_COUNT: usize = 100_000;
const STRUCTURED_EVENT_COUNT: usize = 50_000;
const KEEPALIVE_COUNT: usize = 200_000;

const SMALL_DATA_EVENT: &[u8] = b"data: {\"t\":\"a\"}\n\n";
const MULTILINE_DATA_EVENT: &[u8] = b"data: first\ndata: second\ndata: third\n\n";
const KEEPALIVE: &[u8] = b": keepalive\n\n";

fn split_chunks(bytes: &Bytes, chunk_size: usize) -> Vec<Bytes> {
    (0..bytes.len())
        .step_by(chunk_size)
        .map(|i| bytes.slice(i..bytes.len().min(i + chunk_size)))
        .collect()
}

fn repeated_payload(event: &[u8], event_count: usize) -> Bytes {
    Bytes::from(event.repeat(event_count))
}

fn repeated_event_chunks(event: &[u8], event_count: usize) -> Vec<Bytes> {
    let event = Bytes::copy_from_slice(event);
    (0..event_count).map(|_| event.clone()).collect()
}

fn generate_data_event(payload_size: usize) -> Bytes {
    let payload = "word".repeat(payload_size / 4);
    Bytes::from(format!("data: payload data: {payload}\n\n"))
}

fn generate_metadata_payload(event_count: usize) -> Bytes {
    let mut payload = String::with_capacity(52 * event_count);
    for i in 0..event_count {
        write!(
            &mut payload,
            "id: {i}\nevent: message\ndata: {{\"t\":\"a\"}}\n\n"
        )
        .unwrap();
    }
    Bytes::from(payload)
}

fn bench_small_data(c: &mut Vec<DecodeCase>) {
    let payload = repeated_payload(SMALL_DATA_EVENT, SMALL_EVENT_COUNT);

    add_case(
        c,
        "small_data_whole_stream_chunk",
        vec![payload.clone()],
        SMALL_EVENT_COUNT,
    );
    add_case(
        c,
        "small_data_tcp_chunks",
        split_chunks(&payload, TCP_CHUNK_SIZE),
        SMALL_EVENT_COUNT,
    );
    add_case(
        c,
        "small_data_event_chunks",
        repeated_event_chunks(SMALL_DATA_EVENT, SMALL_EVENT_COUNT),
        SMALL_EVENT_COUNT,
    );
    add_case(
        c,
        "small_data_4b_chunks",
        split_chunks(&payload, PREFIX_SPLIT_CHUNK_SIZE),
        SMALL_EVENT_COUNT,
    );
}

fn bench_structured_events(c: &mut Vec<DecodeCase>) {
    let metadata = generate_metadata_payload(STRUCTURED_EVENT_COUNT);
    add_case(
        c,
        "metadata_events_tcp_chunks",
        split_chunks(&metadata, TCP_CHUNK_SIZE),
        STRUCTURED_EVENT_COUNT,
    );

    let multiline = repeated_payload(MULTILINE_DATA_EVENT, STRUCTURED_EVENT_COUNT);
    add_case(
        c,
        "multiline_data_tcp_chunks",
        split_chunks(&multiline, TCP_CHUNK_SIZE),
        STRUCTURED_EVENT_COUNT,
    );
}

fn bench_keepalives(c: &mut Vec<DecodeCase>) {
    let payload = repeated_payload(KEEPALIVE, KEEPALIVE_COUNT);

    add_case(c, "keepalive_whole_stream_chunk", vec![payload.clone()], 0);
    add_case(
        c,
        "keepalive_tcp_chunks",
        split_chunks(&payload, TCP_CHUNK_SIZE),
        0,
    );
    add_case(
        c,
        "keepalive_line_chunks",
        repeated_event_chunks(KEEPALIVE, KEEPALIVE_COUNT),
        0,
    );
    add_case(
        c,
        "keepalive_4b_chunks",
        split_chunks(&payload, PREFIX_SPLIT_CHUNK_SIZE),
        0,
    );
}

fn bench_large_data(c: &mut Vec<DecodeCase>) {
    let event = generate_data_event(LARGE_PAYLOAD_SIZE);
    let payload = Bytes::from(event.repeat(LARGE_EVENT_COUNT));

    add_case(
        c,
        "large_data_event_chunks",
        (0..LARGE_EVENT_COUNT).map(|_| event.clone()).collect(),
        LARGE_EVENT_COUNT,
    );
    add_case(
        c,
        "large_data_tcp_chunks",
        split_chunks(&payload, TCP_CHUNK_SIZE),
        LARGE_EVENT_COUNT,
    );
}

fn bench_fragmented_medium_data(c: &mut Vec<DecodeCase>) {
    let event = generate_data_event(MEDIUM_PAYLOAD_SIZE);
    let payload = Bytes::from(event.repeat(MEDIUM_EVENT_COUNT));

    add_case(
        c,
        "medium_data_10b_chunks",
        split_chunks(&payload, TINY_CHUNK_SIZE),
        MEDIUM_EVENT_COUNT,
    );
}

pub fn decode_cases() -> Vec<DecodeCase> {
    let mut cases = Vec::new();
    bench_small_data(&mut cases);
    bench_structured_events(&mut cases);
    bench_keepalives(&mut cases);
    bench_large_data(&mut cases);
    bench_fragmented_medium_data(&mut cases);
    add_text_and_mixed_cases(&mut cases);
    cases
}

fn add_text_and_mixed_cases(cases: &mut Vec<DecodeCase>) {
    let json = r#"{"type":"progress","sequence":42,"total":100,"message":"Reading source files","path":"src/stream.rs","done":false}"#;
    for (name, event) in [
        ("json_line", format!("data: {json}\n\n")),
        (
            "json_metadata_after",
            format!("data: {json}\nid: stream/42\n\n"),
        ),
        (
            "json_crlf",
            format!("event: message\r\ndata: {json}\r\nid: stream/42\r\n\r\n"),
        ),
    ] {
        let payload = Bytes::from(event.repeat(20_000));
        add_case(
            cases,
            &format!("{name}_tcp"),
            split_chunks(&payload, TCP_CHUNK_SIZE),
            20_000,
        );
    }

    for (name, text) in [
        ("large_utf8", "中文🙂é".repeat(3334)),
        (
            "large_json_utf8",
            format!(r#"{{"text":"{}"}}"#, "中文内容".repeat(3334)),
        ),
        (
            "large_json_escaped",
            format!(r#"{{"text":"{}"}}"#, r"\u4e2d\u6587".repeat(3334)),
        ),
        (
            "large_json_base64",
            format!(r#"{{"data":"{}"}}"#, "YWJj".repeat(10_000)),
        ),
    ] {
        let event = format!("data: {text}\n\n");
        let payload = Bytes::from(event.repeat(LARGE_EVENT_COUNT));
        add_case(
            cases,
            &format!("{name}_tcp"),
            split_chunks(&payload, TCP_CHUNK_SIZE),
            LARGE_EVENT_COUNT,
        );
        if name == "large_utf8" {
            add_case(
                cases,
                "large_utf8_event",
                repeated_event_chunks(event.as_bytes(), LARGE_EVENT_COUNT),
                LARGE_EVENT_COUNT,
            );
        }
    }

    for (name, big, small) in [
        (
            "mixed_json",
            format!("data: {{\"text\":\"{}\"}}\n\n", "x".repeat(40_000)),
            format!("data: {json}\n\n"),
        ),
        (
            "mixed_multiline",
            format!("data: {}\ndata: tail\n\n", "x".repeat(40_000)),
            "data: x\ndata: y\n\n".into(),
        ),
    ] {
        let cycle = format!("{big}{}", small.repeat(1000));
        let payload = Bytes::from(cycle.repeat(10));
        add_case(
            cases,
            &format!("{name}_tcp"),
            split_chunks(&payload, TCP_CHUNK_SIZE),
            10_010,
        );
        add_case(cases, &format!("{name}_whole"), vec![payload], 10_010);
    }

    let big = format!("data: {{\"text\":\"{}\"}}\n\n", "x".repeat(40_000));
    let small = format!("data: {json}\n\n");
    let alternating = Bytes::from(format!("{big}{small}").repeat(64));
    add_case(
        cases,
        "alternating_sizes_tcp",
        split_chunks(&alternating, TCP_CHUNK_SIZE),
        128,
    );

    // Exercise the copy/transfer boundary with varying payload sizes.
    let mut varied = String::new();
    for _ in 0..8 {
        for size in [32_768, 4096, 8191, 8192, 8193, 512, 16_384, 80] {
            write!(varied, "data: {}\n\n", "x".repeat(size)).unwrap();
        }
    }
    add_case(
        cases,
        "varied_sizes_tcp",
        split_chunks(&Bytes::from(varied), TCP_CHUNK_SIZE),
        64,
    );
    let comments = Bytes::from(b":\r\n".repeat(KEEPALIVE_COUNT));
    add_case(
        cases,
        "comment_crlf_tcp",
        split_chunks(&comments, TCP_CHUNK_SIZE),
        0,
    );
}

pub struct EncodeCase {
    pub name: &'static str,
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
    pub retry: Option<u64>,
}

pub fn encode_cases() -> Vec<EncodeCase> {
    vec![
        EncodeCase {
            name: "small_json",
            event: None,
            data: r#"{"text":"hello","done":false}"#.into(),
            id: None,
            retry: None,
        },
        EncodeCase {
            name: "metadata_json",
            event: Some("message".into()),
            data: r#"{"type":"progress","value":42}"#.into(),
            id: Some("stream/42".into()),
            retry: Some(1000),
        },
        EncodeCase {
            name: "large_ascii",
            event: None,
            data: "x".repeat(40_000),
            id: None,
            retry: None,
        },
        EncodeCase {
            name: "large_utf8",
            event: None,
            data: "中文🙂é".repeat(3334),
            id: None,
            retry: None,
        },
        EncodeCase {
            name: "empty_with_retry",
            event: None,
            data: String::new(),
            id: Some("stream/0".into()),
            retry: Some(u64::MAX),
        },
    ]
}
