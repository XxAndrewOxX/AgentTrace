use anyhow::{Context, Result};
use std::io::{self, Write};
use std::path::Path;
use tracing_subscriber::EnvFilter;

/// Stable user-facing CLI output.
///
/// This is intentionally separate from diagnostic `tracing` events: messages
/// emitted here are part of the command-line contract and must stay script-safe.
pub trait CliOutput: Send + Sync {
    fn line(&self, message: &str) -> Result<()>;
    fn warn(&self, message: &str) -> Result<()>;
    fn error(&self, message: &str) -> Result<()>;
    fn raw_stdout(&self, content: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalOutput {
    quiet: bool,
}

impl TerminalOutput {
    pub fn new(quiet: bool) -> Self {
        Self { quiet }
    }
}

impl CliOutput for TerminalOutput {
    fn line(&self, message: &str) -> Result<()> {
        if self.quiet {
            return Ok(());
        }
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{}", message).context("writing stdout")
    }

    fn warn(&self, message: &str) -> Result<()> {
        let mut stderr = io::stderr().lock();
        writeln!(stderr, "{}", message).context("writing stderr")
    }

    fn error(&self, message: &str) -> Result<()> {
        let mut stderr = io::stderr().lock();
        writeln!(stderr, "{}", message).context("writing stderr")
    }

    fn raw_stdout(&self, content: &str) -> Result<()> {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(content.as_bytes())
            .context("writing raw stdout")?;
        stdout.flush().context("flushing stdout")
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NoopOutput;

impl CliOutput for NoopOutput {
    fn line(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn warn(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn error(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn raw_stdout(&self, _content: &str) -> Result<()> {
        Ok(())
    }
}

pub fn init_tracing(verbosity: u8) -> Result<()> {
    let default_level = match verbosity {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(filter)
        .try_init()
        .map_err(|e| anyhow::anyhow!("initializing tracing subscriber: {}", e))
}

pub fn format_permission_denied(path: &Path, reason: &str) -> String {
    format!("Permission denied: {} — {}", path.display(), reason)
}
