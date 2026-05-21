use agent_trace::commands;
use agent_trace::commands::context::ContextCmd;
use agent_trace::commands::model::ModelCmd;
use agent_trace::mcp;
use agent_trace::types::DocType;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// ── Top-level CLI ─────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "agent-trace",
    version,
    about = "Agent Document Manager — git-backed document store with AI integration",
    arg_required_else_help = true,
)]
pub struct Cli {
    /// Optional agent name (overrides agent-lock file).
    #[arg(long, global = true)]
    pub agent: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

// ── Subcommands ───────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialise a new document store in the given directory.
    Init {
        /// Directory to initialise (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Scan existing markdown files and register them as scratch.
        #[arg(long)]
        scan: bool,
    },

    /// Open the interactive TUI for the store at the given path.
    Open {
        /// Store root (defaults to current directory).
        path: Option<PathBuf>,
        /// Specify the current agent name.
        #[arg(long)]
        agent: Option<String>,
        /// Use ASCII-only box drawing characters.
        #[arg(long)]
        ascii: bool,
    },

    /// Show the status of the document store.
    Status {
        /// Store root (defaults to current directory).
        path: Option<PathBuf>,
    },

    /// Rebuild the manifest from git state.
    Repair,

    /// Register a document with the given type.
    Add {
        /// Document type (plan, context, log, reference, scratch).
        doc_type: DocType,
        /// File to register.
        file: PathBuf,
    },

    /// List registered documents.
    Ls {
        /// Filter by document type.
        #[arg(long = "type", value_name = "TYPE")]
        type_filter: Option<DocType>,
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Show detailed information about a document.
    Info {
        /// File to inspect.
        file: PathBuf,
    },

    /// Change a document's type.
    Reclassify {
        /// File to reclassify.
        file: PathBuf,
        /// New document type.
        new_type: DocType,
    },

    /// Remove a document from tracking (file stays on disk).
    Untrack {
        /// File to untrack.
        file: PathBuf,
    },

    /// Delete a document from tracking and disk.
    Rm {
        /// File to delete.
        file: PathBuf,
    },

    /// Grant a temporary write override for a file.
    Unlock {
        /// File to unlock.
        file: PathBuf,
        /// Actor to grant access to ("user" or "agent").
        #[arg(long = "for")]
        for_actor: String,
        /// Override duration in minutes.
        #[arg(long, default_value = "10")]
        duration: u32,
    },

    /// List recent permission violations.
    Violations {
        /// Maximum number of violations to show.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Manage the synthesized context document.
    Context {
        #[command(subcommand)]
        subcommand: ContextCmd,
    },

    /// Show the document change log.
    Log {
        /// Filter to a specific file.
        file: Option<PathBuf>,
        /// Maximum entries to show.
        #[arg(long)]
        limit: Option<usize>,
        /// Filter by actor ("user", "agent", "system").
        #[arg(long)]
        actor: Option<String>,
        /// Filter by document type.
        #[arg(long = "type", value_name = "TYPE")]
        type_filter: Option<DocType>,
    },

    /// Show the diff between versions of a document.
    Diff {
        /// File to diff.
        file: PathBuf,
        /// First version (defaults to previous).
        v1: Option<u32>,
        /// Second version (defaults to current).
        v2: Option<u32>,
    },

    /// Show a document at a specific version.
    Show {
        /// File to show.
        file: PathBuf,
        /// Version number.
        version: u32,
    },

    /// Restore a document to a previous version.
    Restore {
        /// File to restore.
        file: PathBuf,
        /// Version to restore to.
        version: u32,
    },

    /// Find and replace text across documents.
    Replace {
        /// Search string.
        find: String,
        /// Replacement string.
        replace: String,
        /// Filter to a document type.
        #[arg(long = "type", value_name = "TYPE")]
        type_filter: Option<DocType>,
        /// Preview only, do not apply.
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage the local LLM model.
    Model {
        #[command(subcommand)]
        subcommand: ModelCmd,
    },

    /// Register an agent session (writes lock file, no PID required).
    Connect {
        /// Agent name to register.
        name: String,
    },

    /// End the current agent session (removes lock file).
    Disconnect,

    /// Write content to a tracked document with synchronous permission enforcement.
    Write {
        /// File to write (relative to store root).
        file: PathBuf,
        /// Content to write. Reads from stdin if omitted.
        #[arg(long)]
        content: Option<String>,
    },

    /// Start the MCP server on stdio (JSON-RPC 2.0).
    Mcp {
        /// Store root (defaults to current directory).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Agent name for this MCP session.
        #[arg(long)]
        actor: Option<String>,
    },
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path, scan } => commands::init::run(&path, scan),
        Commands::Open { path, agent, ascii } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            commands::open::run(&root, agent.or(cli.agent), ascii)
        }
        Commands::Status { path } => {
            let root = path.unwrap_or_else(|| PathBuf::from("."));
            commands::status::run(&root)
        }
        Commands::Repair => commands::repair::run(&PathBuf::from(".")),
        Commands::Add { doc_type, file } => {
            commands::add::run(&PathBuf::from("."), doc_type, &file)
        }
        Commands::Ls { type_filter, json } => {
            commands::ls::run(&PathBuf::from("."), type_filter.as_ref(), json)
        }
        Commands::Info { file } => commands::info::run(&PathBuf::from("."), &file),
        Commands::Reclassify { file, new_type } => {
            commands::reclassify::run(&PathBuf::from("."), &file, new_type)
        }
        Commands::Untrack { file } => commands::untrack::run(&PathBuf::from("."), &file),
        Commands::Rm { file } => commands::rm::run(&PathBuf::from("."), &file),
        Commands::Unlock { file, for_actor, duration } => {
            commands::unlock::run(&PathBuf::from("."), &file, &for_actor, duration)
        }
        Commands::Violations { limit } => commands::violations::run(&PathBuf::from("."), limit),
        Commands::Context { subcommand } => {
            commands::context::run(&PathBuf::from("."), subcommand)
        }
        Commands::Log { file, limit, actor, type_filter } => commands::log::run(
            &PathBuf::from("."),
            file.as_deref(),
            limit,
            actor.as_deref(),
            type_filter.as_ref(),
        ),
        Commands::Diff { file, v1, v2 } => {
            commands::diff::run(&PathBuf::from("."), &file, v1, v2)
        }
        Commands::Show { file, version } => {
            commands::show::run(&PathBuf::from("."), &file, version)
        }
        Commands::Restore { file, version } => {
            commands::restore::run(&PathBuf::from("."), &file, version)
        }
        Commands::Replace { find, replace, type_filter, dry_run } => commands::replace::run(
            &PathBuf::from("."),
            &find,
            &replace,
            type_filter.as_ref(),
            dry_run,
        ),
        Commands::Model { subcommand } => commands::model::run(subcommand),
        Commands::Connect { name } => {
            commands::connect::run_connect(&PathBuf::from("."), &name)
        }
        Commands::Disconnect => {
            commands::connect::run_disconnect(&PathBuf::from("."))
        }
        Commands::Write { file, content } => {
            commands::write_cmd::run(&PathBuf::from("."), &file, content, cli.agent)
        }
        Commands::Mcp { path, actor } => {
            mcp::server::run(&path, actor)
        }
    }
}
