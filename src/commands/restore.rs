use crate::observability::CliOutput;
use crate::store::Store;
use crate::types::DocType;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, file: &Path, version: u32, output: &dyn CliOutput) -> Result<()> {
    let store = Store::open(store_root)?;
    let doc_type = store
        .manifest
        .find_by_path(file)
        .map(|d| d.doc_type.clone())
        .unwrap_or(DocType::Scratch);
    store.git.restore_file(file, version, doc_type)?;
    output.line(&format!(
        "Restored {} to version {}",
        file.display(),
        version
    ))?;
    Ok(())
}
