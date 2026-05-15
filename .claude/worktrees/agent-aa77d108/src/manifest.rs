use crate::config::StoreInfo;
use crate::types::DocType;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ── Document Entry ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentEntry {
    pub id: String,
    pub path: PathBuf,
    pub doc_type: DocType,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub agent_name: String,
}

impl DocumentEntry {
    pub fn new(path: PathBuf, doc_type: DocType, agent_name: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            path,
            doc_type,
            tags: Vec::new(),
            description: String::new(),
            agent_name: agent_name.to_string(),
        }
    }
}

// ── On-disk TOML structure ───────────────────────────────────────────────────

/// The raw TOML representation persisted to disk.
#[derive(Debug, Serialize, Deserialize)]
struct ManifestFile {
    store: StoreInfo,
    #[serde(default, rename = "document")]
    documents: Vec<DocumentEntry>,
}

// ── Manifest ─────────────────────────────────────────────────────────────────

pub struct Manifest {
    pub store: StoreInfo,
    pub documents: Vec<DocumentEntry>,
    /// path → index into `documents`
    by_path: HashMap<PathBuf, usize>,
    /// id → index into `documents`
    by_id: HashMap<String, usize>,
}

impl Manifest {
    // ── Construction ─────────────────────────────────────────────────────

    fn build_indices(documents: &[DocumentEntry]) -> (HashMap<PathBuf, usize>, HashMap<String, usize>) {
        let mut by_path = HashMap::new();
        let mut by_id = HashMap::new();
        for (i, doc) in documents.iter().enumerate() {
            by_path.insert(doc.path.clone(), i);
            by_id.insert(doc.id.clone(), i);
        }
        (by_path, by_id)
    }

    pub fn from_parts(store: StoreInfo, documents: Vec<DocumentEntry>) -> Self {
        let (by_path, by_id) = Self::build_indices(&documents);
        Self { store, documents, by_path, by_id }
    }

    // ── I/O ──────────────────────────────────────────────────────────────

    pub fn load(store_root: &Path) -> Result<Self> {
        let path = manifest_path(store_root);

        // Clean up any stale tmp file.
        let tmp_path = tmp_manifest_path(store_root);
        if tmp_path.exists() {
            let _ = std::fs::remove_file(&tmp_path);
        }

        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Reading manifest: {}", path.display()))?;
        let file: ManifestFile = toml::from_str(&contents)
            .with_context(|| format!("Parsing manifest: {}", path.display()))?;

        Ok(Self::from_parts(file.store, file.documents))
    }

    pub fn save(&self, store_root: &Path) -> Result<()> {
        let path = manifest_path(store_root);
        let tmp = tmp_manifest_path(store_root);

        let file = ManifestFile {
            store: self.store.clone(),
            documents: self.documents.clone(),
        };
        let contents = toml::to_string_pretty(&file)?;

        // Atomic write: tmp → rename
        std::fs::write(&tmp, &contents)
            .with_context(|| format!("Writing tmp manifest: {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("Renaming manifest tmp to final: {}", path.display()))?;
        Ok(())
    }

    pub fn create_empty(store_info: StoreInfo, store_root: &Path) -> Result<Self> {
        let m = Self::from_parts(store_info, Vec::new());
        m.save(store_root)?;
        Ok(m)
    }

    // ── CRUD ─────────────────────────────────────────────────────────────

    /// Register a new document. Returns an error if the path is already tracked.
    pub fn register(&mut self, path: &Path, doc_type: DocType, agent_name: &str) -> Result<&DocumentEntry> {
        if self.by_path.contains_key(path) {
            bail!("Path already tracked: {}", path.display());
        }
        let entry = DocumentEntry::new(path.to_path_buf(), doc_type, agent_name);
        let idx = self.documents.len();
        self.by_path.insert(entry.path.clone(), idx);
        self.by_id.insert(entry.id.clone(), idx);
        self.documents.push(entry);
        Ok(&self.documents[idx])
    }

    pub fn find_by_path(&self, path: &Path) -> Option<&DocumentEntry> {
        self.by_path.get(path).map(|&i| &self.documents[i])
    }

    #[allow(dead_code)]
    pub fn find_by_id(&self, id: &str) -> Option<&DocumentEntry> {
        self.by_id.get(id).map(|&i| &self.documents[i])
    }

    pub fn reclassify(&mut self, path: &Path, new_type: DocType) -> Result<()> {
        let idx = *self.by_path.get(path)
            .with_context(|| format!("Path not tracked: {}", path.display()))?;
        self.documents[idx].doc_type = new_type;
        Ok(())
    }

    pub fn update_path(&mut self, old_path: &Path, new_path: &Path) -> Result<()> {
        let idx = *self.by_path.get(old_path)
            .with_context(|| format!("Old path not tracked: {}", old_path.display()))?;
        self.by_path.remove(old_path);
        self.documents[idx].path = new_path.to_path_buf();
        self.by_path.insert(new_path.to_path_buf(), idx);
        Ok(())
    }

    pub fn untrack(&mut self, path: &Path) -> Result<()> {
        let idx = *self.by_path.get(path)
            .with_context(|| format!("Path not tracked: {}", path.display()))?;
        let id = self.documents[idx].id.clone();
        self.by_path.remove(path);
        self.by_id.remove(&id);
        self.documents.remove(idx);
        // Rebuild indices since indices shifted after removal.
        let (by_path, by_id) = Self::build_indices(&self.documents);
        self.by_path = by_path;
        self.by_id = by_id;
        Ok(())
    }

    pub fn list(&self, type_filter: Option<&DocType>) -> Vec<&DocumentEntry> {
        self.documents.iter()
            .filter(|d| type_filter.is_none_or(|t| &d.doc_type == t))
            .collect()
    }

    pub fn is_tracked(&self, path: &Path) -> bool {
        self.by_path.contains_key(path)
    }
}

fn manifest_path(store_root: &Path) -> PathBuf {
    store_root.join(".agent-trace").join("manifest.toml")
}

fn tmp_manifest_path(store_root: &Path) -> PathBuf {
    store_root.join(".agent-trace").join(".manifest.toml.tmp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StoreInfo;
    use tempfile::TempDir;

    fn make_store(tmp: &TempDir) -> (PathBuf, StoreInfo) {
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join(".agent-trace")).unwrap();
        let info = StoreInfo::new("test".into());
        (root, info)
    }

    #[test]
    fn test_empty_manifest_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let m = Manifest::create_empty(info, &root).unwrap();
        assert!(m.documents.is_empty());

        let loaded = Manifest::load(&root).unwrap();
        assert!(loaded.documents.is_empty());
        assert_eq!(loaded.store.name, "test");
    }

    #[test]
    fn test_register_and_lookup() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();

        let path = PathBuf::from("prd.md");
        m.register(&path, DocType::Plan, "").unwrap();

        let entry = m.find_by_path(&path).unwrap();
        assert_eq!(entry.doc_type, DocType::Plan);
        assert!(entry.id.parse::<uuid::Uuid>().is_ok());

        let by_id = m.find_by_id(&entry.id.clone()).unwrap();
        assert_eq!(by_id.path, path);
    }

    #[test]
    fn test_register_duplicate_error() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();
        let path = PathBuf::from("notes.md");
        m.register(&path, DocType::Scratch, "").unwrap();
        assert!(m.register(&path, DocType::Scratch, "").is_err());
    }

    #[test]
    fn test_reclassify() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();
        let path = PathBuf::from("notes.md");
        m.register(&path, DocType::Scratch, "").unwrap();
        m.reclassify(&path, DocType::Plan).unwrap();
        assert_eq!(m.find_by_path(&path).unwrap().doc_type, DocType::Plan);
    }

    #[test]
    fn test_update_path() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();
        let old = PathBuf::from("old.md");
        let new = PathBuf::from("new.md");
        m.register(&old, DocType::Plan, "").unwrap();
        m.update_path(&old, &new).unwrap();
        assert!(m.find_by_path(&old).is_none());
        assert!(m.find_by_path(&new).is_some());
    }

    #[test]
    fn test_untrack() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();
        let path = PathBuf::from("notes.md");
        m.register(&path, DocType::Scratch, "").unwrap();
        m.untrack(&path).unwrap();
        assert!(!m.is_tracked(&path));
        assert!(m.documents.is_empty());
    }

    #[test]
    fn test_list_filter() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();
        m.register(&PathBuf::from("prd.md"), DocType::Plan, "").unwrap();
        m.register(&PathBuf::from("notes.md"), DocType::Scratch, "").unwrap();
        m.register(&PathBuf::from("plan2.md"), DocType::Plan, "").unwrap();

        assert_eq!(m.list(None).len(), 3);
        assert_eq!(m.list(Some(&DocType::Plan)).len(), 2);
        assert_eq!(m.list(Some(&DocType::Scratch)).len(), 1);
    }

    #[test]
    fn test_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let mut m = Manifest::create_empty(info, &root).unwrap();
        m.register(&PathBuf::from("prd.md"), DocType::Plan, "agent-x").unwrap();
        m.save(&root).unwrap();

        let loaded = Manifest::load(&root).unwrap();
        assert_eq!(loaded.documents.len(), 1);
        assert_eq!(loaded.documents[0].doc_type, DocType::Plan);
        assert_eq!(loaded.documents[0].agent_name, "agent-x");
    }

    #[test]
    fn test_stale_tmp_cleaned_on_load() {
        let tmp = TempDir::new().unwrap();
        let (root, info) = make_store(&tmp);
        let m = Manifest::create_empty(info, &root).unwrap();
        m.save(&root).unwrap();

        // Simulate a stale tmp file.
        let tmp_path = root.join(".agent-trace").join(".manifest.toml.tmp");
        std::fs::write(&tmp_path, "garbage").unwrap();
        assert!(tmp_path.exists());

        // Load should clean it up.
        Manifest::load(&root).unwrap();
        assert!(!tmp_path.exists());
    }
}
