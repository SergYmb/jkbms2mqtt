use super::super::transport::internals::hex_display;
use super::support::build_fixture::parse_hex_fixture;

const FRAME_03_FIXTURE: &str = include_str!("fixtures/frame_03_device_info.hex");

const PW1_OFFSET: usize = 0x3E;
const PW2_OFFSET: usize = 0x76;
const PW_LEN: usize = 8;
const REDACTED_HEX: &str = "58 58 58 58 58 58 58 58";

fn hex_bytes_at(hex: &str, byte_offset: usize, len: usize) -> String {
    hex.split_whitespace()
        .skip(byte_offset)
        .take(len)
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn hex_display_redacts_frame03_password_positions() {
    let raw = parse_hex_fixture(FRAME_03_FIXTURE);
    assert_eq!(raw.len(), 300);

    let dumped = hex_display(&raw);

    assert_eq!(hex_bytes_at(&dumped, PW1_OFFSET, PW_LEN), REDACTED_HEX);
    assert_eq!(hex_bytes_at(&dumped, PW2_OFFSET, PW_LEN), REDACTED_HEX);
}

#[test]
fn hex_display_redacts_password_even_when_bms_sends_real_bytes() {
    // Emulate a hypothetical live-device capture that leaked a real ASCII password
    // through the fixture path. The formatter must still emit only `58` at both
    // password positions.
    let mut raw = parse_hex_fixture(FRAME_03_FIXTURE);
    raw[PW1_OFFSET..PW1_OFFSET + PW_LEN].copy_from_slice(b"SECR3T!!");
    raw[PW2_OFFSET..PW2_OFFSET + PW_LEN].copy_from_slice(b"PWD98765");

    let dumped = hex_display(&raw);

    assert_eq!(hex_bytes_at(&dumped, PW1_OFFSET, PW_LEN), REDACTED_HEX);
    assert_eq!(hex_bytes_at(&dumped, PW2_OFFSET, PW_LEN), REDACTED_HEX);

    let real_pw1_hex = "53 45 43 52 33 54 21 21"; // "SECR3T!!"
    let real_pw2_hex = "50 57 44 39 38 37 36 35"; // "PWD98765"
    assert!(
        !dumped.contains(real_pw1_hex),
        "raw password bytes leaked into hex dump"
    );
    assert!(
        !dumped.contains(real_pw2_hex),
        "raw password bytes leaked into hex dump"
    );
}

#[test]
fn hex_display_does_not_redact_non_frame03_buffers() {
    // Frame 0x01 (Configuration): magic + type byte differs — no redaction.
    let mut frame01 = vec![0u8; 300];
    frame01[0..4].copy_from_slice(&[0x55, 0xAA, 0xEB, 0x90]);
    frame01[4] = 0x01;
    frame01[PW1_OFFSET..PW1_OFFSET + PW_LEN].copy_from_slice(&[0xAB; PW_LEN]);

    let dumped = hex_display(&frame01);
    assert_eq!(
        hex_bytes_at(&dumped, PW1_OFFSET, PW_LEN),
        "AB AB AB AB AB AB AB AB",
        "non-Frame-0x03 buffers must not be redacted"
    );
}

#[test]
fn hex_display_does_not_mutate_input() {
    let mut raw = parse_hex_fixture(FRAME_03_FIXTURE);
    raw[PW1_OFFSET..PW1_OFFSET + PW_LEN].copy_from_slice(b"SECRET99");
    let before = raw.clone();
    let _ = hex_display(&raw);
    assert_eq!(raw, before, "hex_display must not mutate its input buffer");
}
