pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Hash of whitespace-normalized payload for exact-normalized duplicates.
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

/// Collapse timestamps, hex ids, numbers and paths so two cargo-test logs
/// that only differ in those tokens share a SimHash neighborhood.
pub fn normalize_for_simhash(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit() || (c == b'x' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_hexdigit())
        {
            while i < bytes.len()
                && (bytes[i].is_ascii_hexdigit()
                    || bytes[i] == b':'
                    || bytes[i] == b'-'
                    || bytes[i] == b'.'
                    || bytes[i] == b'x')
            {
                i += 1;
            }
            out.push('#');
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && (bytes[i + 1].is_ascii_alphanumeric() || bytes[i + 1] == b'.')
        {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push_str("/path");
            continue;
        }
        if c.is_ascii_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// 64-bit SimHash of 3-gram shingles. Skip payloads over 2MB (caller).
pub fn simhash64(text: &str) -> u64 {
    let norm = normalize_for_simhash(text);
    let bytes = norm.as_bytes();
    if bytes.len() < 3 {
        let h = blake3::hash(bytes);
        return u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap());
    }
    let mut acc = [0i32; 64];
    let mut i = 0;
    while i + 2 < bytes.len() {
        let mut h = 0xcbf29ce484222325u64;
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        h ^= bytes[i + 1] as u64;
        h = h.wrapping_mul(0x100000001b3);
        h ^= bytes[i + 2] as u64;
        h = h.wrapping_mul(0x100000001b3);
        for b in 0..64 {
            if (h >> b) & 1 == 1 {
                acc[b] += 1;
            } else {
                acc[b] -= 1;
            }
        }
        i += 1;
    }
    let mut out = 0u64;
    for b in 0..64 {
        if acc[b] >= 0 {
            out |= 1 << b;
        }
    }
    out
}

pub fn simhash_bands(hash: u64) -> [u16; 4] {
    [
        (hash & 0xffff) as u16,
        ((hash >> 16) & 0xffff) as u16,
        ((hash >> 32) & 0xffff) as u16,
        ((hash >> 48) & 0xffff) as u16,
    ]
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
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

    #[test]
    fn simhash_ignores_timestamps() {
        let a = "test auth::login ... ok\nfinished in 1.23s\n2026-08-20T10:00:00Z\n";
        let b = "test auth::login ... ok\nfinished in 9.99s\n2026-08-21T11:11:11Z\n";
        let ha = simhash64(a);
        let hb = simhash64(b);
        assert!(hamming_distance(ha, hb) <= 3, "d={}", hamming_distance(ha, hb));
    }
}
