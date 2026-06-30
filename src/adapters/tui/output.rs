use crate::observability::CliOutput;
use anyhow::Result;
use std::sync::{Arc, Mutex};

/// Captures CLI command output for display in the TUI overlay.
#[derive(Debug, Default)]
pub struct BufferOutput {
    pub lines: Arc<Mutex<Vec<String>>>,
}

impl BufferOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take(&self) -> String {
        let lines = self.lines.lock().unwrap();
        lines.join("\n")
    }
}

impl CliOutput for BufferOutput {
    fn line(&self, message: &str) -> Result<()> {
        self.lines.lock().unwrap().push(message.to_string());
        Ok(())
    }

    fn warn(&self, message: &str) -> Result<()> {
        self.lines.lock().unwrap().push(format!("WARN: {message}"));
        Ok(())
    }

    fn error(&self, message: &str) -> Result<()> {
        self.lines.lock().unwrap().push(format!("ERROR: {message}"));
        Ok(())
    }

    fn raw_stdout(&self, content: &str) -> Result<()> {
        self.lines.lock().unwrap().push(content.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_output_collects_lines() {
        let out = BufferOutput::new();
        out.line("hello").unwrap();
        out.warn("careful").unwrap();
        let text = out.take();
        assert!(text.contains("hello"));
        assert!(text.contains("WARN: careful"));
    }
}
