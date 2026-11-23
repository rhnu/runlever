use super::decode_buffer;

#[test]
fn decodes_a_character_split_across_reads() {
    let mut buffer = vec![b'a', 0xF0, 0x9F, 0x99];
    assert_eq!(decode_buffer(&mut buffer, false), "a");

    buffer.extend_from_slice(&[0x82, b'\r', b'\n']);
    assert_eq!(decode_buffer(&mut buffer, false), "🙂\r\n");
    assert!(buffer.is_empty());
}

#[test]
fn replaces_malformed_sequences_and_incomplete_output_at_eof() {
    let mut buffer = vec![0xFF, b'b', 0xE2, 0x82];
    assert_eq!(decode_buffer(&mut buffer, false), "�b");
    assert_eq!(decode_buffer(&mut buffer, true), "�");
    assert!(buffer.is_empty());
}
