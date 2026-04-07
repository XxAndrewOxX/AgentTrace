use crate::git_store::GitStore;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, version: u32) -> Result<()> {
    let git = GitStore::open(store_root)?;
    let content = git.show_file_at_version(file, version)?;
    print!("{}", content);
    Ok(())
}
