# Adversarial Validation

Specification for the adversarial validation suite in
`tests/adversarial_validation.rs`. Each case ID maps to one `#[test]` unless
noted as intentionally skipped.

Run the suite:

```bash
./scripts/run_e2e.sh adversarial
cargo test --test adversarial_validation
```

**Case count:** 35 documented cases; 34 automated tests (FS-2 is documented but
skipped in CI/local runs).

## 1. Race conditions (RC)

| ID | Intent | Setup | Expected behavior |
|----|--------|-------|-------------------|
| RC-1 | Rapid file creation storm | Create 50 `.md` files, single poll cycle | All 50 committed to git (batched); manifest stays curated (no poll auto-register) |
| RC-2 | Modify during poll | Track file, overwrite 20 times, poll | Latest complete content captured; no crash |
| RC-3 | Create then delete | Create file, delete before poll completes | No ghost manifest entry |
| RC-3b | Commit failure handling | Simulate commit failure during poll | Manifest unchanged (poll never auto-registers); no crash |
| RC-4 | Lock + file mod simultaneously | Write lock file and modify doc concurrently | No crash; consistent state |
| RC-5 | Lock removed during processing | Remove lock mid poll cycle | Consistent attribution per cycle |
| RC-6 | Manifest read/write contention | Concurrent manifest readers/writers | Readers always see complete manifest |
| RC-7 | Git commit during active write | Poll while file is being written | Captures partial or complete content; no corruption |

## 2. Permission enforcement (PE)

| ID | Intent | Setup | Expected behavior |
|----|--------|-------|-------------------|
| PE-1 | Rapid protected writes | Agent writes 10× to each of 3 protected docs | All writes reverted |
| PE-2 | Race the revert | Continuous protected writes | System settles after writer stops |
| PE-3 | Override expiry | Time-limited permission override | Allowed before expiry; denied after |
| PE-4 | Faster-than-classification | Agent creates files rapidly | All committed to git as activity; not auto-registered in manifest |

## 3. Git store integrity (GS)

| ID | Intent | Setup | Expected behavior |
|----|--------|-------|-------------------|
| GS-1 | Log performance at scale | 100 commits across 5 files | Log operations within time budget |
| GS-2 | Abrupt process drop | Simulated SIGKILL mid-operation | Git repo consistent after restart |
| GS-3 | Very long paths | 15-level deep paths | All operations succeed |
| GS-4 | Binary in `.md` | Write binary bytes to tracked file | No panic; graceful handling |
| GS-5 | Empty file | Track empty `.md` | Tracked; diff/version deterministic |
| GS-6 | Symlinks | Symlinks inside store | No infinite loops; no outside-store leaks |

## 4. Discovery & context (DC)

| ID | Intent | Setup | Expected behavior |
|----|--------|-------|-------------------|
| DC-1 | AGENT-TRACE.md under load | Rapid doc changes | Index stays consistent with manifest |
| DC-2 | Conflicting documents | Multiple plan docs with conflicting content | Context synthesis includes both |
| DC-3 | Large document set | 50 plan documents | Context synthesis completes without hang |
| DC-4 | Atomic AGENT-TRACE.md write | Read during regeneration | Readers never see partial file |

## 5. TUI stress (TS)

| ID | Intent | Setup | Expected behavior |
|----|--------|-------|-------------------|
| TS-1 | Changelog eviction | Fill ChangelogState beyond limit | Old entries evicted; bounded memory |
| TS-2 | Long filenames in tree | Very long path names | `render_widget` does not panic |
| TS-3 | Large tree | 500 files in tree panel | Renders without panic or excessive delay |
| TS-4 | Long chat input | 500-character input | ChatState handles without overflow |
| TS-5 | Rapid keyboard input | Burst of key events | ChatState stays consistent |

## 6. Filesystem edge cases (FS)

| ID | Intent | Setup | Expected behavior | Test |
|----|--------|-------|-------------------|------|
| FS-1 | Read-only store | chmod store root 0555 before poll | No panic; graceful error | Yes (Unix) |
| FS-2 | Disk full | Fill filesystem during write | Graceful failure | **Skipped** — requires tmpfs/root; not portable in CI |
| FS-3 | External permission change | chmod file 0000 | Poll handles unreadable file; stays tracked | Yes (Unix) |
| FS-4 | Store moved while running | rename store directory mid-session | Error detected; no panic | Yes |
| FS-5 | `.gitignore` modified | Add ignore rule while running | Well-defined behavior; no crash | Yes |

## 7. Data integrity (DI)

| ID | Intent | Setup | Expected behavior |
|----|--------|-------|-------------------|
| DI-1 | Random operations + repair | 1000 mixed ops, then repair | Manifest matches git state |
| DI-2 | Monotonic versions | Multiple commits to one file | Version numbers monotonic; all retrievable |
| DI-3 | Rename preserves history | Rename tracked file | Full history under new name |
| DI-4 | Restore isolation | Restore older version | Subsequent versions independently retrievable |

## FS-2 exclusion rationale

Disk-full simulation requires filling a mount point (often tmpfs with size limits
or root privileges). macOS CI runners do not expose a reliable, safe disk-full
fixture. FS-2 remains documented for manual validation on controlled Linux
environments but is intentionally excluded from automated runs.
