use crate::observability::CliOutput;
use crate::store::Store;
use anyhow::Result;
use std::path::Path;

pub fn run(
    store_root: &Path,
    file: &Path,
    v1: Option<u32>,
    v2: Option<u32>,
    output: &dyn CliOutput,
) -> Result<()> {
    let store = Store::open(store_root)?;
    let diff = store.git.diff_file(file, v1, v2)?;
    output.line(&diff)?;
    Ok(())
}
