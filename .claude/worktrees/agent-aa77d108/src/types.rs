use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// The semantic type of a document, which determines write permissions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum DocType {
    Plan,
    Context,
    Log,
    Reference,
    Scratch,
}

impl fmt::Display for DocType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocType::Plan => write!(f, "plan"),
            DocType::Context => write!(f, "context"),
            DocType::Log => write!(f, "log"),
            DocType::Reference => write!(f, "reference"),
            DocType::Scratch => write!(f, "scratch"),
        }
    }
}

impl FromStr for DocType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "plan" => Ok(DocType::Plan),
            "context" => Ok(DocType::Context),
            "log" => Ok(DocType::Log),
            "reference" => Ok(DocType::Reference),
            "scratch" => Ok(DocType::Scratch),
            other => Err(anyhow::anyhow!("Unknown doc type: '{}'. Valid types: plan, context, log, reference, scratch", other)),
        }
    }
}

impl DocType {
    /// Short indicator letter for TUI/display.
    pub fn indicator(&self) -> &'static str {
        match self {
            DocType::Plan => "P",
            DocType::Context => "C",
            DocType::Log => "L",
            DocType::Reference => "R",
            DocType::Scratch => "S",
        }
    }
}

/// Who performed an action on a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Actor {
    User,
    Agent { name: String },
    System,
}

impl fmt::Display for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Actor::User => write!(f, "user"),
            Actor::Agent { name } => write!(f, "agent:{}", name),
            Actor::System => write!(f, "system"),
        }
    }
}

impl Actor {
    /// Returns the git author name for commit attribution.
    pub fn git_author_name(&self) -> String {
        match self {
            Actor::User => "User".to_string(),
            Actor::Agent { name } => format!("Agent: {}", name),
            Actor::System => "agent-trace".to_string(),
        }
    }

    /// Returns the git author email for commit attribution.
    pub fn git_author_email(&self) -> &'static str {
        match self {
            Actor::User => "user@agent-trace",
            Actor::Agent { .. } => "agent@agent-trace",
            Actor::System => "system@agent-trace",
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, Actor::Agent { .. })
    }

    pub fn agent_name(&self) -> Option<&str> {
        match self {
            Actor::Agent { name } => Some(name.as_str()),
            _ => None,
        }
    }
}

/// The type of action performed on a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Create,
    Modify,
    Delete,
    Rename,
    Restore,
    Violation,
    Init,
    Unknown,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Create => write!(f, "create"),
            Action::Modify => write!(f, "modify"),
            Action::Delete => write!(f, "delete"),
            Action::Rename => write!(f, "rename"),
            Action::Restore => write!(f, "restore"),
            Action::Violation => write!(f, "violation"),
            Action::Init => write!(f, "init"),
            Action::Unknown => write!(f, "unknown"),
        }
    }
}

impl FromStr for Action {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "create" => Ok(Action::Create),
            "modify" => Ok(Action::Modify),
            "delete" => Ok(Action::Delete),
            "rename" => Ok(Action::Rename),
            "restore" => Ok(Action::Restore),
            "violation" => Ok(Action::Violation),
            "init" => Ok(Action::Init),
            _ => Ok(Action::Unknown),
        }
    }
}

/// A detected change in the working tree.
#[derive(Debug, Clone)]
pub enum FileChange {
    New(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

impl FileChange {
    pub fn path(&self) -> &PathBuf {
        match self {
            FileChange::New(p) | FileChange::Modified(p) | FileChange::Deleted(p) => p,
            FileChange::Renamed { to, .. } => to,
        }
    }

    pub fn action(&self) -> Action {
        match self {
            FileChange::New(_) => Action::Create,
            FileChange::Modified(_) => Action::Modify,
            FileChange::Deleted(_) => Action::Delete,
            FileChange::Renamed { .. } => Action::Rename,
        }
    }
}

/// A parsed git log entry for a document change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub commit_id: String,
    pub timestamp: DateTime<Utc>,
    pub action: Action,
    pub actor: Actor,
    pub agent_name: Option<String>,
    pub files: Vec<(PathBuf, Action, DocType)>,
    pub summary: String,
}

/// Diff statistics.
#[derive(Debug, Clone, Default)]
pub struct DiffStats {
    pub lines_added: usize,
    pub lines_removed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doc_type_roundtrip() {
        for (s, t) in &[
            ("plan", DocType::Plan),
            ("context", DocType::Context),
            ("log", DocType::Log),
            ("reference", DocType::Reference),
            ("scratch", DocType::Scratch),
        ] {
            assert_eq!(s.parse::<DocType>().unwrap(), *t);
            assert_eq!(t.to_string(), *s);
        }
    }

    #[test]
    fn test_doc_type_unknown() {
        assert!("bogus".parse::<DocType>().is_err());
    }

    #[test]
    fn test_actor_git_attribution() {
        assert_eq!(Actor::User.git_author_name(), "User");
        assert_eq!(Actor::User.git_author_email(), "user@agent-trace");

        let agent = Actor::Agent { name: "claude-code".into() };
        assert_eq!(agent.git_author_name(), "Agent: claude-code");
        assert_eq!(agent.git_author_email(), "agent@agent-trace");

        assert_eq!(Actor::System.git_author_name(), "agent-trace");
        assert_eq!(Actor::System.git_author_email(), "system@agent-trace");
    }

    #[test]
    fn test_doc_type_serde() {
        let t = DocType::Plan;
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, r#""plan""#);
        let parsed: DocType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn test_action_display() {
        assert_eq!(Action::Create.to_string(), "create");
        assert_eq!(Action::Modify.to_string(), "modify");
        assert_eq!("modify".parse::<Action>().unwrap(), Action::Modify);
    }
}
