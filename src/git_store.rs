use crate::types::{Action, Actor, CommitId, DocType, DiffStats, FileChange, LogEntry};

type ParsedCommit = (Action, String, Actor, Option<String>, Vec<(PathBuf, Action, DocType)>);
use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use git2::{
    DiffOptions, Oid, Repository, RepositoryInitOptions,
    Signature, StatusOptions, Tree,
};
use std::path::{Path, PathBuf};

pub struct CommitInfo {
    pub action: Action,
    pub files: Vec<(PathBuf, Action, DocType)>,
    pub actor: Actor,
    pub summary: String,
    pub agent_name: Option<String>,
    pub session_id: Option<String>,
}

pub struct GitStore {
    repo: Repository,
    /// The working directory (store root).
    pub workdir: PathBuf,
}

impl GitStore {
    // ── Init / Open ───────────────────────────────────────────────────────

    pub fn init(store_root: &Path) -> Result<Self> {
        let git_dir = store_root.join(".agent-trace").join("repo");

        let mut opts = RepositoryInitOptions::new();
        opts.bare(false);
        opts.workdir_path(store_root);
        opts.no_reinit(false);

        let repo = Repository::init_opts(&git_dir, &opts)
            .with_context(|| format!("Initialising git repo at {}", git_dir.display()))?;

        // Write exclude file so .agent-trace itself is never tracked.
        let exclude = git_dir.join("info").join("exclude");
        std::fs::create_dir_all(exclude.parent().unwrap())?;
        std::fs::write(&exclude, ".agent-trace/\n")?;

        let store = Self { repo, workdir: store_root.to_path_buf() };

        // Create initial empty commit.
        store.create_empty_commit("agent-trace store initialized")?;

        Ok(store)
    }

    pub fn open(store_root: &Path) -> Result<Self> {
        let git_dir = store_root.join(".agent-trace").join("repo");
        if !git_dir.exists() {
            bail!("Not an agent-trace store: .agent-trace/repo not found in {}", store_root.display());
        }
        let repo = Repository::open(&git_dir)
            .with_context(|| format!("Opening git repo at {}", git_dir.display()))?;
        Ok(Self { repo, workdir: store_root.to_path_buf() })
    }

    fn create_empty_commit(&self, message: &str) -> Result<Oid> {
        let sig = Signature::now("agent-trace", "system@agent-trace")?;
        let tree_oid = {
            let mut index = self.repo.index()?;
            index.write_tree()?
        };
        let tree = self.repo.find_tree(tree_oid)?;

        let oid = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            message,
            &tree,
            &[], // no parents for root commit
        )?;
        Ok(oid)
    }

    // ── Status Detection ─────────────────────────────────────────────────

    pub fn detect_changes(&self) -> Result<Vec<FileChange>> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .exclude_submodules(true)
            .renames_from_rewrites(true)
            .renames_index_to_workdir(true)
            .renames_head_to_index(true);

        let statuses = self.repo.statuses(Some(&mut opts))?;
        let mut changes = Vec::new();

        for entry in statuses.iter() {
            let s = entry.status();

            // Rename: has both head_to_index and index_to_workdir paths.
            if s.is_index_renamed() || s.is_wt_renamed() {
                let new_path = match entry.path() {
                    Some(p) => PathBuf::from(p),
                    None => continue,
                };
                let old_path = entry
                    .head_to_index()
                    .and_then(|d| d.old_file().path())
                    .or_else(|| entry.index_to_workdir().and_then(|d| d.old_file().path()))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| new_path.clone());

                if !is_md(&new_path) && !is_md(&old_path) {
                    continue;
                }
                changes.push(FileChange::Renamed { from: old_path, to: new_path });
                continue;
            }

            let path = match entry.path() {
                Some(p) => PathBuf::from(p),
                None => continue,
            };

            if !is_md(&path) {
                continue;
            }

            if s.is_wt_new() || s.is_index_new() {
                changes.push(FileChange::New(path));
            } else if s.is_wt_modified() || s.is_index_modified() {
                changes.push(FileChange::Modified(path));
            } else if s.is_wt_deleted() || s.is_index_deleted() {
                changes.push(FileChange::Deleted(path));
            }
        }

        Ok(changes)
    }

    // ── Commit Operations ─────────────────────────────────────────────────

    pub fn commit(&self, info: &CommitInfo) -> Result<Oid> {
        let mut index = self.repo.index()?;

        for (path, action, _doc_type) in &info.files {
            match action {
                Action::Delete => {
                    if let Err(e) = index.remove_path(path) {
                        tracing::warn!("Could not remove {} from index: {}", path.display(), e);
                    }
                }
                _ => {
                    index.add_path(path)
                        .with_context(|| format!("Staging {}", path.display()))?;
                }
            }
        }
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;

        let parent_commit = self.head_commit()?;
        let parents = vec![&parent_commit];

        let author_name = info.actor.git_author_name();
        let author_email = info.actor.git_author_email();
        let sig = Signature::now(&author_name, author_email)?;

        let message = build_commit_message(info);

        let oid = self.repo.commit(
            Some("HEAD"),
            &sig,
            &sig,
            &message,
            &tree,
            &parents,
        )?;
        Ok(oid)
    }

    fn head_commit(&self) -> Result<git2::Commit<'_>> {
        let head = self.repo.head()?;
        let commit = head.peel_to_commit()?;
        Ok(commit)
    }

    // ── Log / History ─────────────────────────────────────────────────────

    pub fn log(&self, limit: usize) -> Result<Vec<LogEntry>> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let mut entries = Vec::new();
        for oid_result in walk {
            if entries.len() >= limit {
                break;
            }
            let oid = oid_result?;
            let commit = self.repo.find_commit(oid)?;
            if let Some(entry) = parse_commit(&commit) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn log_file(&self, path: &Path, limit: usize) -> Result<Vec<LogEntry>> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let path_str = path.to_string_lossy().to_string();
        let mut entries = Vec::new();

        for oid_result in walk {
            if entries.len() >= limit {
                break;
            }
            let oid = oid_result?;
            let commit = self.repo.find_commit(oid)?;

            // Check if this commit touches the file.
            if !commit_touches_file(&self.repo, &commit, &path_str)? {
                continue;
            }
            if let Some(entry) = parse_commit(&commit) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    #[allow(dead_code)]
    pub fn version_count(&self, path: &Path) -> Result<u32> {
        Ok(self.count_file_commits(path)? as u32)
    }

    /// Count the number of commits that touched `path` without loading them into memory.
    pub fn count_file_commits(&self, path: &Path) -> Result<usize> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        let path_str = path.to_string_lossy().to_string();
        let mut count = 0usize;

        for oid_result in walk {
            let oid = oid_result?;
            let commit = self.repo.find_commit(oid)?;
            if commit_touches_file(&self.repo, &commit, &path_str)? {
                count += 1;
            }
        }
        Ok(count)
    }

    // ── Diff Operations ───────────────────────────────────────────────────

    /// Diff a file between versions. None = working tree / latest commit.
    pub fn diff_file(&self, path: &Path, v1: Option<u32>, v2: Option<u32>) -> Result<String> {
        let path_str = path.to_string_lossy().to_string();

        let (old_tree, new_tree) = self.resolve_version_trees(path, v1, v2)?;

        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(&path_str);

        let diff = self.repo.diff_tree_to_tree(
            old_tree.as_ref(),
            new_tree.as_ref(),
            Some(&mut diff_opts),
        )?;

        let mut output = String::new();
        diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
            let origin = line.origin();
            let content = std::str::from_utf8(line.content()).unwrap_or("");
            match origin {
                '+' | '-' | ' ' => output.push(origin),
                _ => {}
            }
            output.push_str(content);
            true
        })?;

        if output.is_empty() {
            output = "No changes".to_string();
        }
        Ok(output)
    }

    pub fn diff_stats(&self, path: &Path, v1: Option<u32>, v2: Option<u32>) -> Result<DiffStats> {
        let path_str = path.to_string_lossy().to_string();
        let (old_tree, new_tree) = self.resolve_version_trees(path, v1, v2)?;

        let mut diff_opts = DiffOptions::new();
        diff_opts.pathspec(&path_str);

        let diff = self.repo.diff_tree_to_tree(
            old_tree.as_ref(),
            new_tree.as_ref(),
            Some(&mut diff_opts),
        )?;

        let stats = diff.stats()?;
        Ok(DiffStats {
            lines_added: stats.insertions(),
            lines_removed: stats.deletions(),
        })
    }

    pub fn show_file_at_version(&self, path: &Path, version: u32) -> Result<String> {
        let history = self.log_file(path, usize::MAX)?;
        if version == 0 || version as usize > history.len() {
            bail!("Version {} does not exist for {}", version, path.display());
        }
        // history is newest-first; version 1 = oldest
        let idx = history.len() - version as usize;
        let commit_id = &history[idx].commit_id;
        let oid = Oid::from_str(&commit_id.0)?;
        let commit = self.repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let path_str = path.to_string_lossy();
        let entry = tree.get_path(Path::new(path_str.as_ref()))
            .with_context(|| format!("File {} not found at v{}", path.display(), version))?;
        let blob = self.repo.find_blob(entry.id())?;
        Ok(std::str::from_utf8(blob.content())?.to_string())
    }

    /// Resolve (old_tree, new_tree) for a diff. Returns Option<Tree> so None means the working tree.
    fn resolve_version_trees(
        &self,
        path: &Path,
        v1: Option<u32>,
        v2: Option<u32>,
    ) -> Result<(Option<Tree<'_>>, Option<Tree<'_>>)> {
        let history = self.log_file(path, usize::MAX)?;
        let n = history.len();

        let tree_at = |v: u32| -> Result<Tree<'_>> {
            if v == 0 || v as usize > n {
                bail!("Version {} does not exist", v);
            }
            let idx = n - v as usize;
            let oid = Oid::from_str(&history[idx].commit_id.0)?;
            let commit = self.repo.find_commit(oid)?;
            Ok(commit.tree()?)
        };

        let old = match v1 {
            Some(v) => Some(tree_at(v)?),
            None => {
                // Default: compare latest commit with working tree (None means worktree).
                if n == 0 {
                    None
                } else {
                    Some(tree_at(n as u32)?)
                }
            }
        };

        let new = match v2 {
            Some(v) => Some(tree_at(v)?),
            None => None, // working tree
        };

        Ok((old, new))
    }

    // ── Restore / Revert ──────────────────────────────────────────────────

    pub fn restore_file(&self, path: &Path, version: u32, doc_type: DocType) -> Result<Oid> {
        let content = self.show_file_at_version(path, version)?;
        let full_path = self.workdir.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full_path, &content)?;

        let info = CommitInfo {
            action: Action::Restore,
            files: vec![(path.to_path_buf(), Action::Modify, doc_type)],
            actor: Actor::System,
            summary: format!("restore {}: from version {}", path.display(), version),
            agent_name: None,
            session_id: None,
        };
        self.commit(&info)
    }

    pub fn revert_file(&self, path: &Path) -> Result<()> {
        let head = self.head_commit()?;
        let tree = head.tree()?;
        let path_str = path.to_string_lossy();
        let entry = tree.get_path(Path::new(path_str.as_ref()))
            .with_context(|| format!("File {} not found in HEAD", path.display()))?;
        let blob = self.repo.find_blob(entry.id())?;
        let full_path = self.workdir.join(path);
        std::fs::write(&full_path, blob.content())?;
        Ok(())
    }

    /// Return all .md file paths tracked in the git HEAD tree (relative to workdir).
    pub fn head_md_files(&self) -> Result<Vec<PathBuf>> {
        let head = self.head_commit()?;
        let tree = head.tree()?;
        let mut paths = Vec::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let name = entry.name().unwrap_or("");
                if name.ends_with(".md") {
                    let rel = if root.is_empty() {
                        PathBuf::from(name)
                    } else {
                        PathBuf::from(root).join(name)
                    };
                    paths.push(rel);
                }
            }
            git2::TreeWalkResult::Ok
        })?;
        Ok(paths)
    }

    pub fn save_rejected(&self, path: &Path, content: &str) -> Result<()> {
        let rejected_dir = self.workdir.join(".agent-trace").join("rejected");
        std::fs::create_dir_all(&rejected_dir)?;
        let filename = format!(
            "{}-{}.rejected",
            path.file_stem().unwrap_or_default().to_string_lossy(),
            chrono::Utc::now().timestamp()
        );
        std::fs::write(rejected_dir.join(filename), content)?;
        Ok(())
    }
}

// ── Commit message helpers ────────────────────────────────────────────────────

fn build_commit_message(info: &CommitInfo) -> String {
    // Subject: [agent-trace] <action> <type>: <file>
    let first_file = info.files.first();
    let subject = if let Some((path, _action, doc_type)) = first_file {
        format!(
            "[agent-trace] {} {}: {}",
            info.action,
            doc_type,
            path.display()
        )
    } else {
        format!("[agent-trace] {}", info.action)
    };

    let mut body = format!("summary: {}\n", info.summary);
    body.push_str(&format!("actor: {}\n", info.actor));
    if let Some(agent) = &info.agent_name {
        body.push_str(&format!("agent: {}\n", agent));
    }
    if let Some(session) = &info.session_id {
        body.push_str(&format!("session: {}\n", session));
    }
    for (path, action, doc_type) in &info.files {
        body.push_str(&format!("file:\t{}\t{}\t{}\n", path.display(), action, doc_type));
    }

    format!("{}\n\n{}", subject, body)
}

fn parse_commit(commit: &git2::Commit<'_>) -> Option<LogEntry> {
    let message = commit.message().unwrap_or("");
    let timestamp = Utc.timestamp_opt(commit.time().seconds(), 0).single()?;
    let commit_id = CommitId(commit.id().to_string());

    // Try to parse structured message.
    let lines: Vec<&str> = message.lines().collect();
    let subject = lines.first().unwrap_or(&"");

    let (action, summary, actor, agent_name, files) = if subject.starts_with("[agent-trace]") {
        parse_structured_message(message)
    } else {
        (Action::Unknown, message.to_string(), Actor::System, None, Vec::new())
    };

    Some(LogEntry {
        commit_id,
        timestamp,
        action,
        actor,
        agent_name,
        files,
        summary,
    })
}

fn parse_structured_message(message: &str) -> ParsedCommit {
    let mut action = Action::Unknown;
    let mut summary = message.to_string();
    let mut actor = Actor::System;
    let mut agent_name: Option<String> = None;
    let mut files = Vec::new();

    let parts: Vec<&str> = message.splitn(2, "\n\n").collect();
    let subject = parts[0];
    let body = parts.get(1).copied().unwrap_or("");

    // Parse action from subject: "[agent-trace] modify plan: prd.md"
    if let Some(rest) = subject.strip_prefix("[agent-trace] ") {
        let first_word = rest.split_whitespace().next().unwrap_or("");
        action = first_word.parse().unwrap_or(Action::Unknown);
    }

    for line in body.lines() {
        if let Some(val) = line.strip_prefix("summary: ") {
            summary = val.to_string();
        } else if let Some(val) = line.strip_prefix("actor: ") {
            actor = parse_actor_str(val);
        } else if let Some(val) = line.strip_prefix("agent: ") {
            agent_name = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("file:") {
            if let Some(entry) = parse_file_line(val) {
                files.push(entry);
            }
        }
    }

    (action, summary, actor, agent_name, files)
}

fn parse_actor_str(s: &str) -> Actor {
    if s == "user" {
        Actor::User
    } else if s == "system" {
        Actor::System
    } else if let Some(name) = s.strip_prefix("agent:") {
        Actor::Agent { name: name.to_string() }
    } else {
        Actor::System
    }
}

fn parse_file_line(s: &str) -> Option<(PathBuf, Action, DocType)> {
    // Tab-delimited: "{path}\t{action}\t{doc_type}"
    // Note: the "file:\t" prefix is already stripped by strip_prefix("file: ") ... wait,
    // actually strip_prefix("file: ") won't match "file:\t". The caller uses
    // strip_prefix("file: ") but we changed the format to "file:\t". We need to handle
    // the raw value after the "file:\t" prefix, which means s here is already the remainder.
    // The format written is "file:\t{path}\t{action}\t{doc_type}\n"
    // The body line is "file:\t{path}\t{action}\t{doc_type}"
    // strip_prefix("file: ") won't match, so we handle stripping in parse_structured_message.
    // s here is whatever comes after "file:" is stripped — so s = "\t{path}\t{action}\t{doc_type}"
    // We split on '\t', skip the first empty/whitespace element from the leading tab.
    let s = s.trim_start_matches('\t');
    let parts: Vec<&str> = s.splitn(3, '\t').collect();
    if parts.len() < 3 {
        return None;
    }
    let path = PathBuf::from(parts[0]);
    let action = parts[1].parse().unwrap_or(Action::Unknown);
    let doc_type = parts[2].trim_end().parse().unwrap_or(DocType::Scratch);
    Some((path, action, doc_type))
}

fn is_md(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()) == Some("md")
}

fn commit_touches_file(repo: &Repository, commit: &git2::Commit<'_>, path: &str) -> Result<bool> {
    let tree = commit.tree()?;
    let parent_tree: Option<Tree<'_>> = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };

    let mut diff_opts = DiffOptions::new();
    diff_opts.pathspec(path);

    let diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(&mut diff_opts),
    )?;

    Ok(diff.deltas().count() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_store() -> (TempDir, GitStore) {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".agent-trace")).unwrap();
        let store = GitStore::init(tmp.path()).unwrap();
        (tmp, store)
    }

    fn write_md(store: &GitStore, name: &str, content: &str) -> PathBuf {
        let path = store.workdir.join(name);
        std::fs::write(&path, content).unwrap();
        PathBuf::from(name)
    }

    fn commit_file(store: &GitStore, rel_path: &Path, action: Action) {
        let info = CommitInfo {
            action: action.clone(),
            files: vec![(rel_path.to_path_buf(), action, DocType::Plan)],
            actor: Actor::System,
            summary: format!("test commit {}", rel_path.display()),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();
    }

    #[test]
    fn test_init_creates_repo() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".agent-trace")).unwrap();
        let store = GitStore::init(tmp.path()).unwrap();
        assert!(tmp.path().join(".agent-trace").join("repo").exists());
        assert_eq!(store.workdir, tmp.path());
    }

    #[test]
    fn test_open_nonexistent_fails() {
        let tmp = TempDir::new().unwrap();
        assert!(GitStore::open(tmp.path()).is_err());
    }

    #[test]
    fn test_detect_changes_new_md() {
        let (_tmp, store) = setup_store();
        write_md(&store, "notes.md", "# hello");
        let changes = store.detect_changes().unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0], FileChange::New(_)));
    }

    #[test]
    fn test_detect_changes_no_non_md() {
        let (_tmp, store) = setup_store();
        std::fs::write(store.workdir.join("file.txt"), "ignored").unwrap();
        let changes = store.detect_changes().unwrap();
        assert!(changes.is_empty());
    }

    #[test]
    fn test_detect_changes_modified() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "plan.md", "v1");
        commit_file(&store, &rel, Action::Create);
        std::fs::write(store.workdir.join("plan.md"), "v2").unwrap();
        let changes = store.detect_changes().unwrap();
        assert!(changes.iter().any(|c| matches!(c, FileChange::Modified(_))));
    }

    #[test]
    fn test_commit_attribution() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "prd.md", "content");
        let info = CommitInfo {
            action: Action::Create,
            files: vec![(rel.clone(), Action::Create, DocType::Plan)],
            actor: Actor::Agent { name: "claude-code".into() },
            summary: "add prd".into(),
            agent_name: Some("claude-code".into()),
            session_id: None,
        };
        store.commit(&info).unwrap();

        let head = store.head_commit().unwrap();
        assert_eq!(head.author().name().unwrap(), "Agent: claude-code");
        assert_eq!(head.author().email().unwrap(), "agent@agent-trace");
    }

    #[test]
    fn test_log_returns_entries() {
        let (_tmp, store) = setup_store();
        let r1 = write_md(&store, "a.md", "a");
        commit_file(&store, &r1, Action::Create);
        let r2 = write_md(&store, "b.md", "b");
        commit_file(&store, &r2, Action::Create);

        let entries = store.log(10).unwrap();
        // root commit + 2 file commits
        assert!(entries.len() >= 2);
    }

    #[test]
    fn test_log_file_filters() {
        let (_tmp, store) = setup_store();
        let r1 = write_md(&store, "prd.md", "v1");
        commit_file(&store, &r1, Action::Create);
        let r2 = write_md(&store, "other.md", "x");
        commit_file(&store, &r2, Action::Create);
        std::fs::write(store.workdir.join("prd.md"), "v2").unwrap();
        commit_file(&store, &r1, Action::Modify);

        let entries = store.log_file(&PathBuf::from("prd.md"), 10).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_version_count() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "prd.md", "v1");
        commit_file(&store, &rel, Action::Create);
        std::fs::write(store.workdir.join("prd.md"), "v2").unwrap();
        commit_file(&store, &rel, Action::Modify);
        assert_eq!(store.version_count(&PathBuf::from("prd.md")).unwrap(), 2);
    }

    #[test]
    fn test_show_file_at_version() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "prd.md", "version one");
        commit_file(&store, &rel, Action::Create);
        std::fs::write(store.workdir.join("prd.md"), "version two").unwrap();
        commit_file(&store, &rel, Action::Modify);

        let v1 = store.show_file_at_version(&PathBuf::from("prd.md"), 1).unwrap();
        assert_eq!(v1.trim(), "version one");
        let v2 = store.show_file_at_version(&PathBuf::from("prd.md"), 2).unwrap();
        assert_eq!(v2.trim(), "version two");
    }

    #[test]
    fn test_revert_file() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "prd.md", "original");
        commit_file(&store, &rel, Action::Create);
        std::fs::write(store.workdir.join("prd.md"), "unauthorized change").unwrap();
        store.revert_file(&PathBuf::from("prd.md")).unwrap();
        let content = std::fs::read_to_string(store.workdir.join("prd.md")).unwrap();
        assert_eq!(content.trim(), "original");
    }

    #[test]
    fn test_commit_message_format() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "prd.md", "content");
        let info = CommitInfo {
            action: Action::Modify,
            files: vec![(rel.clone(), Action::Modify, DocType::Plan)],
            actor: Actor::User,
            summary: "updated prd".into(),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();
        let head = store.head_commit().unwrap();
        let msg = head.message().unwrap();
        assert!(msg.contains("[agent-trace] modify plan: prd.md"), "Got: {}", msg);
        assert!(msg.contains("actor: user"));
    }

    // ── parse_file_line unit tests ──────────────────────────────────────────

    #[test]
    fn test_parse_file_line_normal() {
        // Round-trip: path without spaces
        let s = "\tprd.md\tmodify\tplan";
        let result = parse_file_line(s);
        assert!(result.is_some(), "Expected Some, got None");
        let (path, action, doc_type) = result.unwrap();
        assert_eq!(path, PathBuf::from("prd.md"));
        assert!(matches!(action, Action::Modify), "action = {:?}", action);
        assert!(matches!(doc_type, DocType::Plan), "doc_type = {:?}", doc_type);
    }

    #[test]
    fn test_parse_file_line_path_with_spaces() {
        // Path containing spaces must round-trip correctly with tab delimiter
        let s = "\tmy plan.md\tcreate\tplan";
        let result = parse_file_line(s);
        assert!(result.is_some(), "Expected Some for path-with-spaces, got None");
        let (path, action, doc_type) = result.unwrap();
        assert_eq!(path, PathBuf::from("my plan.md"));
        assert!(matches!(action, Action::Create), "action = {:?}", action);
        assert!(matches!(doc_type, DocType::Plan), "doc_type = {:?}", doc_type);
    }

    #[test]
    fn test_parse_file_line_unknown_action() {
        // Unknown action string should fall back to Action::Unknown gracefully
        let s = "\tnotes.md\tfrob\tscratch";
        let result = parse_file_line(s);
        assert!(result.is_some(), "Expected Some even with unknown action");
        let (path, action, _doc_type) = result.unwrap();
        assert_eq!(path, PathBuf::from("notes.md"));
        assert!(matches!(action, Action::Unknown), "Expected Unknown, got {:?}", action);
    }

    #[test]
    fn test_parse_file_line_unknown_doc_type() {
        // Unknown doc_type string should fall back to DocType::Scratch gracefully
        let s = "\tnotes.md\tmodify\tfluxcapacitor";
        let result = parse_file_line(s);
        assert!(result.is_some(), "Expected Some even with unknown doc_type");
        let (_path, _action, doc_type) = result.unwrap();
        assert!(matches!(doc_type, DocType::Scratch), "Expected Scratch fallback, got {:?}", doc_type);
    }

    #[test]
    fn test_parse_file_line_roundtrip_via_commit() {
        // Full round-trip: write a commit with a path containing spaces, read it back
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "my plan.md", "content with spaces in name");
        let info = CommitInfo {
            action: Action::Create,
            files: vec![(rel.clone(), Action::Create, DocType::Plan)],
            actor: Actor::User,
            summary: "add my plan".into(),
            agent_name: None,
            session_id: None,
        };
        store.commit(&info).unwrap();
        let entries = store.log_file(&rel, 5).unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.files.len(), 1);
        assert_eq!(entry.files[0].0, PathBuf::from("my plan.md"));
        assert!(matches!(entry.files[0].2, DocType::Plan));
    }

    #[test]
    fn test_count_file_commits() {
        let (_tmp, store) = setup_store();
        let rel = write_md(&store, "prd.md", "v1");
        commit_file(&store, &rel, Action::Create);
        std::fs::write(store.workdir.join("prd.md"), "v2").unwrap();
        commit_file(&store, &rel, Action::Modify);
        let count = store.count_file_commits(&PathBuf::from("prd.md")).unwrap();
        assert_eq!(count, 2);
    }
}
