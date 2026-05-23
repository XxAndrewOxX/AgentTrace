use crate::observability::CliOutput;
use crate::store::Store;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, version: u32, output: &dyn CliOutput) -> Result<()> {
    let store = Store::open(store_root)?;
    let content = store.git.show_file_at_version(file, version)?;
    output.raw_stdout(&content)?;
    Ok(())
}
