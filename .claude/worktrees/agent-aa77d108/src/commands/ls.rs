use crate::manifest::Manifest;
use crate::types::DocType;
use anyhow::Result;
use std::path::Path;

pub fn run(store_root: &Path, type_filter: Option<&DocType>, json: bool) -> Result<()> {
    let manifest = Manifest::load(store_root)?;
    let docs = manifest.list(type_filter);

    if json {
        let entries: Vec<serde_json::Value> = docs.iter().map(|d| {
            serde_json::json!({
                "id": d.id,
                "path": d.path,
                "doc_type": d.doc_type.to_string(),
                "tags": d.tags,
                "description": d.description,
                "agent_name": d.agent_name,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if docs.is_empty() {
        println!("No documents tracked.");
        return Ok(());
    }

    for doc in docs {
        println!("[{}] {}", doc.doc_type.indicator(), doc.path.display());
    }
    Ok(())
}
