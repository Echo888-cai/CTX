use crate::exec;
use anyhow::Context;

/// Run a command through CTX and print a markdown report with ctx:// links.
pub fn run(shell: bool, command: Vec<String>) -> anyhow::Result<()> {
    let delivered = exec::capture(shell, None, &command).context("ctx exec")?;
    let uris: Vec<&str> = delivered
        .split_whitespace()
        .filter(|w| w.starts_with("ctx://"))
        .collect();
    println!("## CTX");
    println!();
    if uris.is_empty() {
        println!("Command finished. No pages were virtualized (output was under the threshold).");
    } else {
        println!("Raw tool output stayed in the local store. Page in with `ctx fetch <uri>`.");
        println!();
        for uri in &uris {
            println!("- `{uri}`");
        }
    }
    println!();
    println!("```");
    println!("{}", delivered.trim_end());
    println!("```");
    Ok(())
}
