/// Strip ANSI / CSI / OSC sequences and carriage returns from terminal output.
pub fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    if !bytes.contains(&0x1b) && !bytes.contains(&b'\r') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i] != 0x1b && bytes[i] != b'\r' {
            i += 1;
        }
        if i > start {
            out.push_str(&input[start..i]);
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'\r' {
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            break;
        }
        match bytes[i] {
            b'[' => {
                i += 1;
                while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            b']' => {
                i += 1;
                while i < bytes.len() && bytes[i] != 0x07 && bytes[i] != 0x1b {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == 0x07 {
                    i += 1;
                } else if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'\\' {
                    i += 2;
                }
            }
            b'(' | b')' => i += 2,
            _ => i += 1,
        }
    }
    out
}

const SPINNER: &[char] = &[
    '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏', '◐', '◓', '◑', '◒',
];

pub fn is_progress_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if t.chars().any(|c| SPINNER.contains(&c)) {
        return true;
    }
    if t.contains('%') && (t.contains('[') || t.contains('=') || t.contains('#')) {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("downloading") && (lower.contains("mb") || lower.contains("%")) {
        return true;
    }
    if regex_is_bar(t) {
        return true;
    }
    false
}

fn regex_is_bar(t: &str) -> bool {
    // Pytest/cargo banners are mostly '=' but contain words. Real progress
    // bars are almost entirely bar characters.
    let letters = t.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if letters > 6 {
        return false;
    }
    let stripped: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if stripped.len() < 8 {
        return false;
    }
    let bars = stripped
        .chars()
        .filter(|c| matches!(c, '=' | '#' | '█' | '▓' | '▒' | '-' | '▌'))
        .count();
    bars * 2 > stripped.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_color() {
        let s = "\x1b[31mFAIL\x1b[0m ok";
        assert_eq!(strip_ansi(s), "FAIL ok");
    }

    #[test]
    fn detects_spinner() {
        assert!(is_progress_line("⠋ compiling..."));
        assert!(!is_progress_line("error: missing type"));
    }

    #[test]
    fn pytest_banner_is_not_a_bar() {
        assert!(!is_progress_line(
            "=================================== FAILURES ==================================="
        ));
        assert!(!is_progress_line(
            "=========================== short test summary info ============================"
        ));
        assert!(is_progress_line(
            "tests/test_health.py ........................................           [  4%]"
        ));
    }
}
