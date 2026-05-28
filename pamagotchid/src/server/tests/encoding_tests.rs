use super::*;

#[test]
fn decode_base64_accepts_plain_and_data_url_inputs() {
    assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
    assert_eq!(
        decode_base64("data:image/png;base64,aGVsbG8=").unwrap(),
        b"hello"
    );
}
