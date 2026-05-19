use crate::store::Store;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, v1: Option<u32>, v2: Option<u32>) -> Result<()> {
    let store = Store::open(store_root)?;
    let output = store.git.diff_file(file, v1, v2)?;
    println!("{}", output);
    Ok(())
}
