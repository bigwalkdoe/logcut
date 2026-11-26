use std::fs;
use std::io::{self, IsTerminal, Read, Write};
#[cfg(feature = "cli")]
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use logcut::core::{
    self,
    models::{
        SliceBuildArgs, SliceBuildReport, SliceContainerRequest, SliceLogRequest, SliceReport,
        SliceServiceRequest,
    },
};

#[derive(Parser)]
#[command(
    name = "logcut",
    version,
    about = "Local-first stacktrace and runtime log slicer"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Print JSON statistics to stderr
    #[arg(long, global = true)]
    stats: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Slice a log file, piped output, or inline text
    Slice(SliceArgs),
    /// Collapse compiler error cascades from a build log or piped output
    Build(BuildArgs),
    /// Slice Docker container logs
    Docker(DockerArgs),
    /// Slice a systemd unit's journal
    Journal(JournalArgs),
    /// Ask an OpenAI-compatible model using logcut tools
    Ask(AskArgs),
    /// Serve the MCP protocol over stdio
    Mcp,
    /// Install MCP config and /debug-log commands for AI hosts
    Init(InitArgs),
}

#[derive(Args)]
struct SliceArgs {
    file: Option<PathBuf>,
    #[arg(long)]
    raw_text: Option<String>,
    #[arg(long, default_value = ".")]
    workspace_root: String,
    #[arg(long)]
    tail_lines: Option<usize>,
    #[arg(long)]
    context_lines: Option<usize>,
    #[arg(long)]
    include_vendor_frames: bool,
    #[arg(long)]
    max_chars: Option<usize>,
}

#[derive(Args)]
struct BuildArgs {
    file: Option<PathBuf>,
    #[arg(long)]
    workspace_root: Option<String>,
    #[arg(long)]
    max_errors: Option<usize>,
    #[arg(long)]
    context_lines: Option<usize>,
    #[arg(long)]
    max_chars: Option<usize>,
}

#[derive(Args)]
struct DockerArgs {
    container: String,
    #[arg(long)]
    tail_lines: Option<usize>,
    #[arg(long)]
    since: Option<String>,
    #[arg(long, default_value = ".")]
    workspace_root: String,
    #[arg(long)]
    max_chars: Option<usize>,
}

#[derive(Args)]
struct JournalArgs {
    unit: String,
    #[arg(long)]
    lines: Option<usize>,
    #[arg(long, default_value = ".")]
    workspace_root: String,
    #[arg(long)]
    max_chars: Option<usize>,
}

#[derive(Args)]
struct AskArgs {
    question: Vec<String>,
    #[arg(long, default_value = ".")]
    workspace_root: String,
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    opencode: bool,
    #[arg(long)]
    claude: bool,
    #[arg(long)]
    cursor: bool,
    #[arg(long)]
    all: bool,
}

fn emit(report: SliceReport, show_stats: bool) -> Result<()> {
    if show_stats {
        writeln!(io::stderr(), "{}", serde_json::to_string(&report.stats)?)?;
    }
    println!("{}", report.text);
    Ok(())
}

fn emit_build(report: SliceBuildReport, show_stats: bool) -> Result<()> {
    if show_stats {
        writeln!(io::stderr(), "{}", serde_json::to_string(&report.stats)?)?;
    }
    println!("{}", report.text);
    Ok(())
}

fn piped_stdin() -> Result<Option<String>> {
    if io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut text = String::new();
    io::stdin().read_to_string(&mut text)?;
    Ok(Some(text))
}

const SLASH_COMMAND: &str = "---\ndescription: Slice logs and stacktraces with logcut\n---\n\
Inspect $ARGUMENTS: pipe failing command output or a log file into `logcut slice`, \
then summarize the surviving stack frames and deduplicated error bursts. \
Use `logcut mcp` tools when available.\n";

fn init_documents(args: &InitArgs) -> Vec<(&'static str, String)> {
    let server = r#"{"type":"stdio","command":"logcut","args":["mcp"],"description":"Run logcut mcp over stdio"}"#;
    let mut documents = Vec::new();
    let opencode = args.opencode || args.all;
    let claude = args.claude || args.all;
    let cursor = args.cursor || args.all;

    if opencode {
        documents.push((
            "opencode.json",
            r#"{"$schema":"https://opencode.ai/config.json","mcp":{"servers":{"logcut":{"type":"local","command":["logcut","mcp"]}}}}"#.to_owned(),
        ));
        documents.push((".opencode/commands/debug-log.md", SLASH_COMMAND.to_owned()));
    }
    if claude {
        documents.push((
            ".mcp.json",
            format!(r#"{{"mcpServers":{{"logcut":{server}}}}}"#),
        ));
        documents.push((".claude/commands/debug-log.md", SLASH_COMMAND.to_owned()));
    }
    if cursor {
        documents.push((
            ".cursor/mcp.json",
            format!(r#"{{"mcpServers":{{"logcut":{server}}}}}"#),
        ));
        documents.push((
            ".cursor/rules/debug-log.mdc",
            format!(
                "---\ndescription: Debug logs with logcut\nalwaysApply: false\n---\n{SLASH_COMMAND}"
            ),
        ));
    }
    documents
}

fn write_documents(documents: &[(&str, String)]) -> Result<()> {
    let base = std::env::current_dir()?;
    for (relative, contents) in documents {
        let path = base.join(relative);
        if path.exists() {
            eprintln!("logcut init: keeping existing {}", path.display());
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)?;
        eprintln!("logcut init: wrote {}", path.display());
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Diagnostics never share stdout with JSON-RPC or piped reports.
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(io::stderr)
        .with_max_level(tracing::Level::INFO)
        .init();

    match cli.command {
        Commands::Mcp => {
            #[cfg(feature = "mcp")]
            {
                // Only JSON-RPC may reach stdout in this mode.
                logcut::mcp::serve().await?;
                Ok(())
            }
            #[cfg(not(feature = "mcp"))]
            anyhow::bail!("this build does not include the mcp feature")
        }
        Commands::Slice(args) => {
            let piped =
                if args.file.is_none() && args.raw_text.is_none() && !io::stdin().is_terminal() {
                    let mut text = String::new();
                    io::stdin().read_to_string(&mut text)?;
                    Some(text)
                } else {
                    None
                };
            let report = core::slice_log(SliceLogRequest {
                path: args.file.map(|path| path.to_string_lossy().into_owned()),
                raw_text: args.raw_text.or(piped),
                workspace_root: Some(args.workspace_root),
                tail_lines: args.tail_lines,
                context_lines: args.context_lines,
                include_vendor_frames: Some(args.include_vendor_frames),
                max_chars: args.max_chars,
            })?;
            emit(report, cli.stats)
        }
        Commands::Build(args) => {
            let raw_text = if args.file.is_none() {
                piped_stdin()?
            } else {
                None
            };
            let report = core::slice_build(SliceBuildArgs {
                path: args.file.map(|path| path.to_string_lossy().into_owned()),
                raw_text,
                workspace_root: args.workspace_root,
                max_errors: args.max_errors,
                context_lines: args.context_lines,
                max_chars: args.max_chars,
            })?;
            emit_build(report, cli.stats)
        }
        Commands::Docker(args) => {
            let report = core::slice_container(SliceContainerRequest {
                container: args.container,
                tail_lines: args.tail_lines,
                since: args.since,
                workspace_root: Some(args.workspace_root),
                max_chars: args.max_chars,
            })
            .await?;
            emit(report, cli.stats)
        }
        Commands::Journal(args) => {
            let report = core::slice_service(SliceServiceRequest {
                unit: args.unit,
                lines: args.lines,
                workspace_root: Some(args.workspace_root),
                max_chars: args.max_chars,
            })
            .await?;
            emit(report, cli.stats)
        }
        Commands::Ask(args) => {
            #[cfg(feature = "cli")]
            {
                let answer =
                    logcut::agent::ask(&args.question.join(" "), Path::new(&args.workspace_root))
                        .await?;
                println!("{answer}");
                Ok(())
            }
            #[cfg(not(feature = "cli"))]
            {
                let _ = args;
                anyhow::bail!("this build does not include the cli feature")
            }
        }
        Commands::Init(args) => {
            let args = if !args.opencode && !args.claude && !args.cursor && !args.all {
                InitArgs {
                    opencode: true,
                    claude: true,
                    cursor: true,
                    all: true,
                }
            } else {
                args
            };
            write_documents(&init_documents(&args))
        }
    }
}
