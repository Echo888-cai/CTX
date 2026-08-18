mod app;
mod demo;
mod doctor;
mod exec;
mod inspect;
mod search;
mod setup;
mod status;
mod why;

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use ctx_core::{handle_hook, CtxPaths, Runtime, Snapshot};
use ctx_protocol::{CtxEvent, Harness, ToolRef};

#[derive(Parser)]
#[command(
    name = "ctx",
    version,
    about = "Virtual memory for AI context.",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create ~/.ctx and detect harnesses
    Init,
    /// Show today's context efficiency
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Check binary, store, hooks, MCP
    Doctor,
    /// Install hooks + MCP for a harness
    Setup {
        /// claude | cursor | all
        target: String,
    },
    /// Run a command, store raw output, print the working set
    Exec {
        /// Run through `sh -c`
        #[arg(long)]
        shell: bool,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Explain today's token reductions
    Why,
    /// Show HOT / WARM / COLD (working-set clock)
    Inspect {
        #[arg(long)]
        session: Option<String>,
        /// Rank mapped pages by task tokens
        #[arg(long)]
        task: Option<String>,
    },
    /// Search stored pages (page-fault retrieval)
    Search {
        query: String,
        #[arg(long, short, default_value_t = 8)]
        limit: usize,
    },
    /// Page in a stored payload
    Fetch {
        uri: String,
        #[arg(long, short)]
        query: Option<String>,
        /// Return the full stored page (skip the working-set preview)
        #[arg(long)]
        full: bool,
    },
    /// List named frames (virtual addresses) on a page
    Frames { uri: String },
    /// Read a file through CTX
    Read {
        path: PathBuf,
        #[arg(long, short)]
        query: Option<String>,
    },
    /// Harness hook entry (stdin JSON → stdout JSON)
    Hook,
    /// Reduce stdin as shell output (dogfood)
    Reduce,
    /// Run a fixture through CTX (first-run page fault)
    Demo,
    /// Pause virtualization (tool output passes through)
    #[command(visible_alias = "off")]
    Pause,
    /// Resume virtualization
    #[command(visible_alias = "on")]
    Resume,
    /// Open the local dashboard (today's avoided tokens)
    App {
        #[arg(long, default_value_t = 8741)]
        port: u16,
        /// Do not open a browser
        #[arg(long)]
        no_open: bool,
        /// Start the dashboard at login (macOS LaunchAgent / systemd user)
        #[arg(long)]
        install_service: bool,
        /// Remove the login dashboard
        #[arg(long)]
        uninstall_service: bool,
    },
    /// Serve the CTX MCP (stdio)
    Mcp,
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive(
                "ctx=info"
                    .parse()
                    .unwrap_or_else(|_| "info".parse().unwrap()),
            ),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("ctx: {err:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None => print_home(),
        Some(Commands::Init) => setup::init(),
        Some(Commands::Status { json }) => status::run(json),
        Some(Commands::Doctor) => doctor::run(),
        Some(Commands::Setup { target }) => setup::setup(&target),
        Some(Commands::Exec {
            shell,
            cwd,
            command,
        }) => {
            let code = exec::run(shell, cwd.as_deref(), &command)?;
            std::process::exit(code);
        }
        Some(Commands::Why) => why::run(),
        Some(Commands::Inspect { session, task }) => {
            inspect::run(session.as_deref(), task.as_deref())
        }
        Some(Commands::Search { query, limit }) => search::run(&query, limit),
        Some(Commands::Fetch { uri, query, full }) => {
            let rt = Runtime::open_default().context("open CTX store")?;
            let q = if full { Some("*".to_string()) } else { query };
            let out = rt.fetch(&uri, q.as_deref())?;
            print!("{out}");
            if !out.ends_with('\n') {
                println!();
            }
            Ok(())
        }
        Some(Commands::Frames { uri }) => {
            let rt = Runtime::open_default().context("open CTX store")?;
            let parsed = ctx_protocol::CtxUri::parse(&uri).context("ctx:// URI")?;
            let frames = rt.store.frames_for(&parsed.page_key())?;
            if frames.is_empty() {
                println!("No frames on {}", parsed.page_key());
                println!("Ingest a test/build log first (ctx demo).");
                return Ok(());
            }
            println!("Address space  {}", parsed.page_key());
            println!();
            for f in frames {
                let addr = parsed.clone().with_frame(&f.name);
                println!(
                    "  {addr:<44}  {:<6}  L{}–{}  {}",
                    f.kind, f.start_line, f.end_line, f.hint
                );
            }
            Ok(())
        }
        Some(Commands::Read { path, query }) => {
            let rt = Runtime::open_default().context("open CTX store")?;
            let out = rt.read_file(&path.to_string_lossy(), query.as_deref())?;
            print!("{out}");
            if !out.ends_with('\n') {
                println!();
            }
            Ok(())
        }
        Some(Commands::Hook) => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let rt = match Runtime::open_default() {
                Ok(rt) => rt,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "ctx hook skipped — store unavailable. Tool continues (fail-open)."
                    );
                    return Ok(());
                }
            };
            let resp = handle_hook(&rt, &buf);
            let mut out = std::io::stdout();
            out.write_all(resp.stdout.as_bytes())?;
            Ok(())
        }
        Some(Commands::Reduce) => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let rt = Runtime::open_default().context("open CTX store")?;
            let event =
                CtxEvent::tool_output("reduce", Harness::Unknown, ToolRef::new("Bash"), buf);
            let result = rt.ingest(event)?;
            print!("{}", result.delivered);
            if !result.delivered.ends_with('\n') {
                println!();
            }
            eprintln!(
                "raw {}  delivered {}  avoided {}  ({})",
                result.raw_tokens,
                result.delivered_tokens,
                result.avoided_tokens,
                result.optimizer.as_deref().unwrap_or("passthrough")
            );
            Ok(())
        }
        Some(Commands::Demo) => demo::run(),
        Some(Commands::Pause) => status::set_enabled(false),
        Some(Commands::Resume) => status::set_enabled(true),
        Some(Commands::App {
            port,
            no_open,
            install_service,
            uninstall_service,
        }) => app::run(port, !no_open, install_service, uninstall_service),
        Some(Commands::Mcp) => {
            let rt = Runtime::open_default().context("open CTX store")?;
            ctx_mcp::serve(rt)?;
            Ok(())
        }
    }
}

fn print_home() -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let paths = CtxPaths::default_home()?;
    if !paths.db_path().exists() {
        println!("{}", status::home(version, None, true));
        return Ok(());
    }
    match Runtime::open_default() {
        Ok(rt) => {
            let snap = Snapshot::capture(&rt.store)?;
            println!("{}", status::home(version, Some(&snap), rt.config.enabled));
        }
        Err(_) => {
            println!("{}", status::home(version, None, true));
        }
    }
    Ok(())
}
