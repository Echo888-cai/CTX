pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Hash of whitespace-normalized payload for near-duplicate detection.
pub fn normalize_hash(text: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    let mut first = true;
    for word in text.split_whitespace() {
        if !first {
            hasher.update(b" ");
        }
        hasher.update(word.as_bytes());
        first = false;
    }
    hasher.finalize().to_hex().to_string()
}

pub fn blob_relpath(hash: &str) -> std::path::PathBuf {
    let (prefix, rest) = if hash.len() >= 2 {
        (&hash[..2], &hash[2..])
    } else {
        ("00", hash)
    };
    std::path::PathBuf::from(prefix).join(format!("{rest}.zst"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash() {
        assert_eq!(blake3_hex(b"abc"), blake3_hex(b"abc"));
        assert_ne!(blake3_hex(b"abc"), blake3_hex(b"abd"));
    }

    #[test]
    fn whitespace_normalized() {
        assert_eq!(normalize_hash("a  b\n c"), normalize_hash("a b c"));
    }
}
