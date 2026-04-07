/// Shared test helpers for E2E tests.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

pub struct TestStore {
    pub dir: TempDir,
    pub bin: PathBuf,
}

impl TestStore {
    pub fn new() -> Self {
        let dir = TempDir::new().expect("create tempdir");
        let bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/release/docmgr");
        let s = Self { dir, bin };
        s.docmgr(&["init", s.dir.path().to_str().unwrap()])
            .expect_success("init");
        s
    }

    pub fn new_with_scan(files: &[(&str, &str)]) -> Self {
        let dir = TempDir::new().expect("create tempdir");
        let bin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/release/docmgr");
        let s = Self { dir, bin };
        // Create files before init --scan.
        for (name, content) in files {
            let path = s.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
        }
        s.docmgr(&["init", s.dir.path().to_str().unwrap(), "--scan"])
            .expect_success("init --scan");
        s
    }

    /// Run docmgr with given args from the store directory.
    pub fn docmgr(&self, args: &[&str]) -> DocmgrOutput {
        let output = Command::new(&self.bin)
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("run docmgr");
        DocmgrOutput { output }
    }

    /// Run docmgr with --agent flag.
    pub fn docmgr_as_agent<'a>(&self, agent: &'a str, args: &[&str]) -> DocmgrOutput {
        let mut full_args = vec!["--agent", agent];
        full_args.extend_from_slice(args);
        self.docmgr(&full_args)
    }

    pub fn write_file(&self, name: &str, content: &str) {
        let path = self.dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    pub fn read_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).unwrap()
    }

    pub fn file_exists(&self, name: &str) -> bool {
        self.dir.path().join(name).exists()
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }
}

pub struct DocmgrOutput {
    pub output: Output,
}

impl DocmgrOutput {
    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).to_string()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).to_string()
    }

    pub fn success(&self) -> bool {
        self.output.status.success()
    }

    pub fn expect_success(self, ctx: &str) -> Self {
        if !self.output.status.success() {
            panic!(
                "{} failed:\nstdout: {}\nstderr: {}",
                ctx,
                self.stdout(),
                self.stderr()
            );
        }
        self
    }

    pub fn expect_failure(self, ctx: &str) -> Self {
        if self.output.status.success() {
            panic!(
                "{} expected failure but succeeded:\nstdout: {}\nstderr: {}",
                ctx,
                self.stdout(),
                self.stderr()
            );
        }
        self
    }

    pub fn assert_stdout_contains(&self, needle: &str) -> &Self {
        let s = self.stdout();
        assert!(
            s.contains(needle),
            "expected stdout to contain {:?}, got:\n{}",
            needle,
            s
        );
        self
    }

    pub fn assert_stdout_not_contains(&self, needle: &str) -> &Self {
        let s = self.stdout();
        assert!(
            !s.contains(needle),
            "expected stdout NOT to contain {:?}, but got:\n{}",
            needle,
            s
        );
        self
    }
}
