/// Smoke test for deterministic wait helpers (decoupled from synthesis).
#[path = "helpers.rs"]
mod helpers;

use helpers::wait_for_file;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn wait_for_file_detects_delayed_creation() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let rel = "delayed.txt";
    let ready = Arc::new(Mutex::new(false));
    let ready_clone = Arc::clone(&ready);
    let root_clone = root.clone();
    let rel_owned = rel.to_string();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(root_clone.join(&rel_owned), "hello").unwrap();
        *ready_clone.lock().unwrap() = true;
    });

    assert!(wait_for_file(&root, rel, Duration::from_secs(5)));
    assert!(*ready.lock().unwrap());
}
