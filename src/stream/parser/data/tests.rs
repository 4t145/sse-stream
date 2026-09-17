use super::{DataBuffer, MAX_RETAINED_CAPACITY};

#[test]
fn completed_large_event_bounds_retained_scratch_space() {
    let mut buffer = DataBuffer::default();
    let large = vec![b'x'; MAX_RETAINED_CAPACITY * 8];
    buffer.push_fragment(&large);
    buffer.finish_line().expect("ASCII is valid UTF-8");
    assert_eq!(buffer.take().expect("completed data").as_bytes(), large);
    assert!(buffer.bytes.capacity() <= MAX_RETAINED_CAPACITY);
    buffer.push_line(b"small").expect("valid ASCII");
    let small = buffer.take().expect("completed small data");
    assert_eq!(small, "small");
    assert!(small.capacity() <= 64);
}

#[test]
fn unfinished_bytes_never_reach_owned_string() {
    let mut buffer = DataBuffer::default();
    buffer.push_line(b"valid").expect("valid ASCII");
    buffer.push_fragment(b"\xff");
    assert_eq!(buffer.take().as_deref(), Some("valid"));
    buffer.push_fragment(b"\xff");
    buffer.push_line(b"next").expect("valid ASCII");
    assert_eq!(buffer.take().as_deref(), Some("next"));
}

#[test]
fn failed_line_preserves_only_completed_lines() {
    let mut buffer = DataBuffer::default();
    buffer.push_line(b"").expect("empty line is valid");
    buffer.push_fragment(b"\xe4");
    assert!(buffer.finish_line().is_err());
    assert_eq!(buffer.take().as_deref(), Some(""));
    assert!(buffer.take().is_none());
}
