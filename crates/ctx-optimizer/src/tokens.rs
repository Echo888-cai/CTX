/// Honest token estimate. Not a billing number.
///
/// Mixed code and logs sit around 3.8 characters per token. This is labeled
/// "estimated" everywhere it is shown to users.
pub fn estimate_tokens(text: &str) -> u32 {
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
    let words = word_count(text) as f64;
    let by_chars = ascii as f64 / 3.8 + other as f64 / 1.1;
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
        let expected = (chars / 3.8).max(words * 1.3).round().max(1.0) as u32;
        assert_eq!(estimate_tokens(s), expected);
    }
}
