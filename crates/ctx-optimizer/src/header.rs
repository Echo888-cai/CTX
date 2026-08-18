//! `$ command` / `exit N` prefix shared by ShellGuard and `ctx exec`.

pub fn truncate_cmd(cmd: &str, max_chars: usize) -> String {
    if cmd.chars().count() <= max_chars {
        cmd.to_string()
    } else {
        format!(
            "{}…",
            cmd.chars()
                .take(max_chars.saturating_sub(1))
                .collect::<String>()
        )
    }
}

/// Header lines only (`$ cmd\nexit 1\n`). Empty if the body already has them.
pub fn command_exit_header(command: Option<&str>, exit: Option<i64>, body: &str) -> String {
    let mut header = String::new();
    let first = body.lines().next().unwrap_or("");
    if let Some(cmd) = command.map(str::trim).filter(|s| !s.is_empty()) {
        if !first.starts_with("$ ") {
            header.push_str(&format!("$ {}\n", truncate_cmd(cmd, 80)));
        }
    }
    if let Some(code) = exit {
        if !body.lines().any(|l| l.starts_with("exit ")) {
            header.push_str(&format!("exit {code}\n"));
        }
    }
    header
}

pub fn prepend_command_exit(command: Option<&str>, exit: Option<i64>, body: &str) -> String {
    let header = command_exit_header(command, exit, body);
    if header.is_empty() {
        body.to_string()
    } else {
        format!("{header}\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_when_already_present() {
        let body = "$ cargo test\nexit 0\n\nok\n";
        assert_eq!(
            prepend_command_exit(Some("cargo test"), Some(0), body),
            body
        );
    }

    #[test]
    fn adds_both() {
        let out = prepend_command_exit(Some("cargo test"), Some(1), "boom\n");
        assert!(out.starts_with("$ cargo test\nexit 1\n\nboom\n"), "{out}");
    }
}
