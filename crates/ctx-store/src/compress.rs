//! Adaptive zstd. Stream encode when the payload is already large in RAM
//! so the compressor does not hold a second full-size scratch buffer.

use std::io::{self, Write};

const STREAM_AT: usize = 1_048_576;

pub fn level_for(bytes: usize, kind: Option<&str>) -> i32 {
    let kind = kind.unwrap_or("");
    if bytes < 1_024 {
        return 1;
    }
    if bytes > STREAM_AT {
        return 1;
    }
    match kind {
        "file" if bytes >= 100_000 => 5,
        "shell" | "mcp" if bytes <= 100_000 => 3,
        "file" => 3,
        _ if bytes <= 100_000 => 3,
        _ => 3,
    }
}

pub fn encode(bytes: &[u8], kind: Option<&str>) -> io::Result<Vec<u8>> {
    let level = level_for(bytes.len(), kind);
    if bytes.len() >= STREAM_AT {
        let mut enc =
            zstd::stream::Encoder::new(Vec::with_capacity((bytes.len() / 4).max(64)), level)?;
        enc.write_all(bytes)?;
        return enc.finish();
    }
    zstd::encode_all(bytes, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_shell_is_fast() {
        assert_eq!(level_for(200, Some("shell")), 1);
    }

    #[test]
    fn mid_shell_is_balanced() {
        assert_eq!(level_for(50_000, Some("shell")), 3);
    }

    #[test]
    fn large_file_is_denser() {
        assert_eq!(level_for(200_000, Some("file")), 5);
    }

    #[test]
    fn huge_log_stays_fast() {
        assert_eq!(level_for(2_000_000, Some("shell")), 1);
    }

    #[test]
    fn roundtrip_small() {
        let raw = b"ctx virtual memory ".repeat(40);
        let z = encode(&raw, Some("shell")).unwrap();
        let back = zstd::decode_all(z.as_slice()).unwrap();
        assert_eq!(back, raw);
        assert!(z.len() < raw.len());
    }
}
