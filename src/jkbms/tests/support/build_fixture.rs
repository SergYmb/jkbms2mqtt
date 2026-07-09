/// Parse a fixture file: strip `#` comment lines, collect space-separated hex bytes.
pub fn parse_hex_fixture(text: &str) -> Vec<u8> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .flat_map(|l| l.split_whitespace())
        .map(|tok| u8::from_str_radix(tok, 16).expect("invalid hex token in fixture"))
        .collect()
}

/// Recompute and stamp the 8-bit modulo checksum at byte 299 of a 300-byte JK frame.
pub fn stamp_checksum(frame: &mut [u8; 300]) {
    let sum: u32 = frame[..299].iter().map(|&b| u32::from(b)).sum();
    frame[299] = (sum % 256) as u8;
}

/// Build a minimal 300-byte Frame 0x02 with only the bytes needed for a specific test.
/// All other bytes are zero. Checksum is stamped automatically.
pub fn build_frame02(patches: &[(usize, &[u8])]) -> [u8; 300] {
    let mut frame = [0u8; 300];
    frame[0] = 0x55;
    frame[1] = 0xAA;
    frame[2] = 0xEB;
    frame[3] = 0x90;
    frame[4] = 0x02;
    for (offset, bytes) in patches {
        frame[*offset..*offset + bytes.len()].copy_from_slice(bytes);
    }
    stamp_checksum(&mut frame);
    frame
}
