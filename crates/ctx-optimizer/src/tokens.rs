/// Honest token estimate. Not a billing number.
///
/// Mixed code and logs sit around 3.8 characters per token. This is labeled
/// "estimated" everywhere it is shown to users.
pub fn estimate_tokens(text: &str) -> u32 {
    estimate_tokens_for(TokenKind::Shell, text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Code,
    Shell,
    Json,
    Prose,
    Cjk,
    Binary,
}

pub fn sniff_token_kind(tool_kind: &str, text: &str) -> TokenKind {
    let sample = if text.len() > 4000 { &text[..4000] } else { text };
    let non_ascii = sample.chars().filter(|c| !c.is_ascii()).count();
    if !sample.is_empty() && non_ascii * 100 / sample.chars().count().max(1) >= 20 {
        return TokenKind::Cjk;
    }
    let trimmed = sample.trim_start();
    if (trimmed.starts_with('{') || trimmed.starts_with('[')) && sample.contains(':') {
        return TokenKind::Json;
    }
    let b64ish = sample
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/' || *b == b'=')
        .count();
    if sample.len() > 80 && b64ish * 100 / sample.len() > 92 {
        return TokenKind::Binary;
    }
    match tool_kind {
        "file" => TokenKind::Code,
        "mcp" => TokenKind::Json,
        "shell" => TokenKind::Shell,
        _ => TokenKind::Prose,
    }
}

pub fn estimate_tokens_for(kind: TokenKind, text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let mut ascii = 0u32;
    let mut other = 0u32;
    if text.is_ascii() {
        ascii = text.len() as u32;
    } else {
        for c in text.chars() {
            if c.is_ascii() {
                ascii += 1;
            } else {
                other += 1;
            }
        }
    }
    let (ascii_div, other_div) = match kind {
        TokenKind::Code => (3.3, 1.1),
        TokenKind::Shell => (3.9, 1.1),
        TokenKind::Json => (2.9, 1.1),
        TokenKind::Prose => (4.2, 1.1),
        TokenKind::Cjk => (3.8, 0.9),
        TokenKind::Binary => (3.0, 1.1),
    };
    let words = word_count(text) as f64;
    let by_chars = ascii as f64 / ascii_div + other as f64 / other_div;
    let by_words = words * 1.3;
    by_chars.max(by_words).round().max(1.0) as u32
}

fn word_count(text: &str) -> usize {
    if text.is_ascii() {
        let mut n = 0usize;
        let mut in_word = false;
        for &b in text.as_bytes() {
            if b.is_ascii_whitespace() {
                in_word = false;
            } else if !in_word {
                in_word = true;
                n += 1;
            }
        }
        n
    } else {
        text.split_whitespace().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn scales_with_length() {
        let small = estimate_tokens("hello world");
        let big = estimate_tokens(&"hello world ".repeat(1000));
        assert!(big > small * 50);
    }

    #[test]
    fn ascii_matches_unicode_path_on_ascii() {
        let s = "error: boom\nleft: 401\nright: 200\n";
        let chars = s.chars().count() as f64;
        let words = s.split_whitespace().count() as f64;
        let expected = (chars / 3.9).max(words * 1.3).round().max(1.0) as u32;
        assert_eq!(estimate_tokens(s), expected);
    }
}
