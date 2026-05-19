use crate::store::Store;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, version: u32) -> Result<()> {
    let store = Store::open(store_root)?;
    let content = store.git.show_file_at_version(file, version)?;
    print!("{}", content);
    Ok(())
}
