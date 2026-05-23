use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::{
    git::{CommitInfo, GitStore},
    manifest::Manifest,
    permissions::Overrides,
};

/// Bundles GitStore, Manifest, and Overrides for a single store root.
/// Commands should use this instead of opening each component separately.
pub struct Store {
    pub root: PathBuf,
    pub git: GitStore,
    pub manifest: Manifest,
    pub overrides: Overrides,
}

impl Store {
    /// Open an existing store at `root`. Returns an error if not initialized.
    pub fn open(root: &Path) -> Result<Self> {
        let git = GitStore::open(root)
            .with_context(|| format!("Not an agent-trace store: {}", root.display()))?;
        let manifest = Manifest::load(root)
            .with_context(|| format!("Failed to load manifest at {}", root.display()))?;
        let overrides = Overrides::load(root)
            .with_context(|| "Failed to load overrides")?;
        Ok(Self {
            root: root.to_path_buf(),
            git,
            manifest,
            overrides,
        })
    }

    /// Commit changes to the store with a structured commit message.
    pub fn commit(&self, info: &CommitInfo) -> Result<git2::Oid> {
        self.git.commit(info)
    }
}
