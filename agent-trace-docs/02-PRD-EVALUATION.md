# PRD Evaluation: `agent-trace` v0.3.0

**Evaluator:** Independent technical review  
**Date:** 2026-04-01  
**PRD Version Under Review:** 0.3.0-draft  
**Method:** Section-by-section evaluation against clarity, completeness, internal consistency, implementability, and alignment with stated goals.

---

## Evaluation Summary

| Category | Score | Notes |
|----------|-------|-------|
| Problem Clarity | **Strong** | Well-defined gap, clear user need |
| Scope Discipline | **Strong** | Won't-have list is explicit and well-reasoned |
| Internal Consistency | **Needs Work** | Several contradictions between sections (detailed below) |
| Implementability | **Moderate** | Most sections are buildable; a few are under-specified |
| Architecture Fitness | **Moderate** | Good overall, but threading model and TOML write patterns need attention |
| Risk Awareness | **Strong** | Honest about LLM reliability; missing a few risks |
| Testability | **Moderate** | Strategy exists but lacks acceptance criteria tied to requirements |
| First-Run Experience | **Needs Work** | User journey from install to first value is unclear |

**Overall Verdict:** The PRD is ~80% ready for implementation handoff. The remaining 20% consists of internal contradictions, under-specified state transitions, and a missing user journey. These are fixable without structural changes.

---

## Section-by-Section Evaluation

### 1. Executive Summary — PASS

Clear, concise, correctly prioritized. The three key architectural decisions (LLM optional, stat-based polling, TOML metadata) are stated upfront, which is excellent for alignment.

**One concern:** The phrase "local interactive server" in the summary is misleading. `agent-trace` is not a server — it's a TUI application that runs a polling loop. The word "server" implies network listeners, HTTP endpoints, etc. Recommend changing to "local interactive terminal application."

---

### 2. Problem Statement — PASS

Well-articulated gap. Each bullet point maps to a specific capability in the product.

**Missing:** No mention of what users currently do as workarounds. Understanding current behavior (even if it's "nothing" or "manual git diffs") helps implementers understand the bar they need to clear. Even one sentence like "Users currently rely on git diffs and terminal scrollback, which lack document-level semantics" would anchor the problem.

---

### 3. Target Users — PASS with caveat

Three personas are clear. However, the third persona ("Agent framework developers") implies `agent-trace` exposes a programmatic API or protocol that agents can integrate with. The current PRD only offers a file-based lock mechanism (Section 7.2) and the agents write to the filesystem directly. **This persona's need ("standardized document store that agents can read from and write to") is mostly unmet in V1** — agents just write files and `agent-trace` reacts. That's fine, but the persona description oversells V1's agent integration.

**Recommendation:** Either (a) tone down the third persona to "Need a managed folder with semantic metadata that agents can write into," or (b) add a minimal write protocol (e.g., agents can read `manifest.toml` to discover document paths and types).

---

### 4. Core Concepts — MIXED

#### 4.1 Document Store — PASS
Clear definition. Markdown-only scope is well-justified.

#### 4.2 Document Types — PASS
The simplification from 7 to 5 types is a good call. The rationale for dropping `agent-input`/`agent-output` is sound.

**Edge case to clarify:** What type is a document that an agent creates as a log of its own actions? Is that a `log` or a `scratch`? The table says logs can be created by agents, but the description says "human-readable audit trails." If an agent writes raw structured output (not human-readable), is that still a `log`? Recommend adding: "If an agent writes unstructured or machine-readable output, classify as `scratch` until reclassified."

#### 4.3 Hidden Metadata — PASS
Clean layout. Good separation of concerns.

**Issue:** The `file_index.toml` is described here but its full schema isn't shown until Section 4.4. An implementer reading linearly won't understand what it is yet. Consider moving the schema here or adding a forward reference.

#### 4.4 Storage Backend: TOML + JSONL — NEEDS WORK

This is the most critical section and has several issues:

**Issue 1: Manifest rewrite frequency.**
The PRD says "the full manifest is rewritten atomically on every change." In the TUI with 1-second polling, if 5 files change in one poll cycle, the manifest is rewritten 5 times in rapid succession. Each rewrite involves serializing all documents to TOML, writing to a temp file, fsyncing, and renaming.

**Question for the PRD:** Should changes detected in a single poll cycle be batched into one manifest write? This is the obvious answer but it's not stated anywhere. The change processor (Section 7.3) receives events from the poller but the PRD doesn't specify batching.

**Recommendation:** Explicitly state: "All changes detected in a single poll cycle are processed as a batch. The manifest is rewritten once per poll cycle, not once per file change."

**Issue 2: Version history per-document file — write pattern unclear.**
Section 4.6 says version history is stored in `.agent-trace/history/versions/<doc-id>.toml`. When a new version is created, this file must be updated. But the PRD only specifies atomic write patterns for the manifest (Section 7.6), not for version history files. Do these also use write-tmp-rename? If 10 files change in one poll cycle, that's 10 version history files + 1 manifest file + 1 changelog append = 12 file operations.

**Recommendation:** Specify: "Version history files use the same write-tmp-rename pattern as the manifest. All file I/O for a single poll cycle is batched: snapshot writes first, then version history updates, then manifest, then changelog append."

**Issue 3: Changelog JSONL — what is it FOR?**
The manifest is the source of truth. The per-document version files store version history. What does the changelog add? It appears to be a denormalized view that combines all changes across all documents in chronological order — essentially the data source for the TUI's changelog panel and the `agent-trace log` command (without a file filter).

This is fine, but the PRD should explicitly state: "The changelog is a derived/denormalized view for efficient chronological querying. It can be safely deleted and rebuilt from version history files."

**Issue 4: `doc_id` format ambiguity.**
The manifest example shows `id = "doc-001"`. The text says "UUID, survives renames." But `doc-001` is not a UUID. Later, the tech stack lists the `uuid` crate. Which is it? Sequential IDs like `doc-001` require a counter; UUIDs don't.

**Recommendation:** Pick one. UUIDs are simpler (no counter to maintain, no collision risk). Change the example to a real UUID.

#### 4.5 Snapshot Storage — PASS
Content-addressed storage is well-specified. GC strategy is clear.

**Minor gap:** What happens if a snapshot file is missing but referenced by a version entry? This is a corruption scenario. `agent-trace repair` should handle it, but the expected behavior isn't specified. Recommend: "If a snapshot is missing, the version entry is marked as `corrupted` and the user is warned. The document's current on-disk content is always available as the latest version regardless of snapshot state."

#### 4.6 Version Control Model — PASS with issues

The decision to store versions in per-document files is sound for keeping the manifest lean.

**Issue: The version history TOML schema is shown but inconsistent with the manifest.**
The version history uses `[[documents.doc-001.versions]]` syntax, which implies it's nested inside the manifest. But Section 4.4 says version history is in separate files. The example schema should show what a standalone `doc-001.toml` version file looks like:

```toml
[[versions]]
version = 1
content_hash = "sha256:..."
created_at = "2026-03-31T10:05:00Z"
actor = "user"
...
```

Not:

```toml
[[documents.doc-001.versions]]
...
```

This is a copy-paste inconsistency that will confuse implementers.

#### 4.7 Ignore File — PASS
Standard pattern, well-specified.

---

### 5. Core Functionalities — MIXED

#### 5.1 Document Store Management — PASS

**Gap in 5.1.2 (Document Registration):** "Polling detection — `agent-trace` detects new `.md` files during its poll cycle." But the poller (Section 5.2.1) says it calls `stat()` on "every tracked file" and also "scans for new `.md` files." The scan-for-new-files part is a recursive directory walk. For a store with deep nesting, this could be slow.

**Question:** How deep does the recursive scan go? Is there a max depth? Are subdirectories created by agents automatically scanned, or does the user need to opt them in? The PRD doesn't specify.

**Recommendation:** "The poller recursively scans all subdirectories of the store root, excluding `.agent-trace/` and paths matching the ignore file. There is no depth limit. For V1, stores with >1000 files may experience slower poll cycles; this is acceptable."

#### 5.1.4 Agent Log Synthesis — PASS
Good LLM-absent fallback. The "don't re-synthesize if agent wrote directly" heuristic is smart.

**Gap:** How does `agent-trace` decide WHICH log document to append to? If there are multiple `log`-type documents, does it create a new one? Append to the most recent? Use a naming convention?

**Recommendation:** "When synthesizing a log entry, `agent-trace` appends to the log document tagged with the current agent's session ID (from the agent-lock file). If no such document exists, `agent-trace` creates a new log document named `logs/<agent-name>-<session-id>.md`."

#### 5.2 Change Tracking — PASS

The stat-based polling is well-justified and well-specified.

**Issue in rename detection:** "When a file disappears and a new file appears in the same poll cycle, and the new file's content hash matches the disappeared file's last known hash."

This requires reading and hashing the new file to compare. But the stat-based approach only reads files when `mtime` or `size` changes. A newly appeared file has no cached stats, so it WILL be read and hashed. Good — but the PRD should make this explicit: "New files are always fully read and hashed on their first poll cycle."

**Edge case:** What if a file is renamed AND modified in the same poll cycle? The content hash won't match the old file. This should be treated as delete + create, which is the correct behavior. Worth stating explicitly.

#### 5.3 LLM-Powered CLI — PASS

Well-scoped for V1. Limiting LLM to classification, summarization, and command interpretation is the right call.

**Gap in 5.3.2:** The chat interface section doesn't specify what happens when the user types something that's neither a valid structured command nor natural language (e.g., a typo like `lls`). Does it fall through to the LLM? Does it show "Unknown command"?

**Recommendation:** Specify parsing order: (1) Try to parse as a structured command. (2) If parsing fails AND LLM is loaded, send to LLM as natural language. (3) If parsing fails AND LLM is NOT loaded, show "Unknown command. Type `help` for available commands."

---

### 6. TUI — PASS with minor issues

The layout, keyboard shortcuts, and visual design are clear and implementable.

**Issue 1: Panel proportions not specified.** The ASCII mockup shows roughly 40/60 split. But at different terminal widths, what's the ratio? Fixed column width for the tree? Percentage split? The implementer needs to know.

**Recommendation:** "The document tree panel takes 35% of terminal width (minimum 30 columns). The changelog panel takes the remaining width. If the terminal is too narrow for both panels, the changelog panel is hidden and can be toggled with a keybinding."

**Issue 2: What happens when the tree is very deep/wide?** If a store has 10 levels of nesting, the tree panel will overflow horizontally. Is there horizontal scrolling? Truncation?

**Recommendation:** "File paths in the tree panel are truncated with `...` if they exceed the panel width. The tree can be scrolled vertically but not horizontally in V1."

**Issue 3: No specification for the Help overlay.** Section 6.5 says `h` / `?` shows a "Help overlay" but there's no description of what it contains. Even a one-liner: "The help overlay lists all keyboard shortcuts and available structured commands."

---

### 7. Architecture — MIXED

#### 7.1 High-Level Architecture — PASS
Clean layer separation.

#### 7.2 Agent Attribution — PASS
Two portable mechanisms, well-specified.

**Gap:** The agent-lock file specifies a `pid` field and `agent-trace` removes stale locks where the PID no longer exists. How does `agent-trace` check if a PID exists portably? On Linux: check `/proc/<pid>`. On macOS: `kill(pid, 0)`. The `kill` signal-zero approach works on both. Specify: "Stale lock detection uses `kill(pid, 0)` (signal zero) — if the process doesn't exist, the lock is considered stale."

#### 7.3 Async/Threading Architecture — NEEDS WORK

**Critical issue: Three tasks but unclear data flow.**

The PRD defines three background tasks: Stat Poller, LLM Inference, and Change Processor. But the data flow between them is incomplete:

1. **Poller detects change** → sends `FileChange` to Change Processor. Good.
2. **Change Processor creates snapshot, updates manifest.** Good.
3. **Change Processor wants an LLM summary** → sends `LlmRequest` to LLM engine. But **the PRD doesn't show this channel.** The channel list only has `llm_request_tx/rx` going from "command router -> LLM engine." The Change Processor also needs to send LLM requests (for change summarization), but it's not in the channel diagram.

**Recommendation:** Add a channel from Change Processor -> LLM engine, or make the `llm_request_tx` cloneable (which `mpsc` senders are) and give a clone to the Change Processor. Specify: "Both the command router (for NL commands) and the change processor (for change summarization) send requests to the LLM engine via cloned `llm_request_tx` senders."

**Second issue: Blocking on LLM response.**

When the Change Processor sends a summarization request to the LLM, does it block waiting for the response? If so, it can't process other file changes while waiting for the LLM (which may take seconds). If not, how does the summary get attached to the version entry?

**Recommendation:** Specify: "The Change Processor does NOT block on LLM responses. It creates the version entry with an empty `summary` field immediately. When the LLM response arrives, the Change Processor updates the version entry's summary retroactively. The TUI changelog panel initially shows the diff-stats summary and updates to the LLM summary when available."

#### 7.4 LLM Integration — PASS
Prompt patterns are clear and well-budgeted.

#### 7.5 Structured Command Protocol — PASS
The Rust enum is implementation-ready.

#### 7.6 Manifest Write Safety — PASS
Atomic write pattern is correct and well-specified.

**Gap:** No mention of write safety for the `file_index.toml`. This file is updated every poll cycle. Does it also use write-tmp-rename? It probably should — a corrupted file index means the next poll cycle will re-stat everything (slow but not data-losing). State the expected behavior: "If `file_index.toml` is corrupted or missing, the poller rebuilds it by stat-checking all tracked files. This is slower for one cycle but causes no data loss."

---

### 8. CLI Commands Reference — PASS

Comprehensive and well-organized. The clap hierarchy is clear.

**Gap:** No specification for exit codes. Non-interactive commands should return meaningful exit codes for scripting:
- 0: success
- 1: general error
- 2: store not found / not initialized
- 3: file not found / not tracked

---

### 9. Configuration — PASS

Clean hierarchy with global defaults and per-store overrides.

**Gap:** What happens when the global config file doesn't exist? Presumably defaults are used, but state this explicitly.

---

### 10. Non-Functional Requirements — PASS with one concern

Performance targets are realistic and testable.

**Concern:** "Manifest parse time (500 docs) < 100ms" — has this been benchmarked? A 500-document manifest with ~200 bytes per entry is ~100KB of TOML. TOML parsing in Rust (`toml` crate) is fast but 100ms is a specific claim. If this turns out to be 200ms, is that a blocker? Recommend making this a soft target: "Manifest parse time should be under 200ms for 500 documents. If parsing exceeds 500ms, consider migrating to SQLite."

---

### 11. Definition of Done — NEEDS WORK

**Critical problem: No acceptance criteria.**

The DoD is a checklist of features, not a set of verifiable acceptance criteria. For example:

- "Stat-based polling detects file create, modify, and delete" — How is this verified? What's the test scenario? What's the pass condition?
- "Manifest writes are atomic (write-tmp-rename)" — How do you prove atomicity? Kill the process during a write and verify no corruption?

Each Must Have item should have at least one acceptance test scenario. This doesn't need to be in the PRD itself, but there should be a reference to a separate acceptance test document, or the testing strategy (Section 13) should map tests to DoD items.

**Recommendation:** Add a column to the DoD checklist or create a separate acceptance test matrix:

| Requirement | Acceptance Test |
|-------------|-----------------|
| Stat-based polling detects file create | Create a `.md` file in the store. Within 2 poll cycles, `agent-trace status` shows it as tracked. |
| Manifest writes are atomic | `kill -9` the process during a manifest write (simulated). On restart, manifest is either the old version or the new version, never corrupt. |

**Second problem: LLM items are "Must Have" but LLM is "optional."**

Section 11.2 is titled "Must Have — LLM-Enhanced." But the executive summary says "The LLM is an optional enhancement, not a hard dependency." If the LLM is optional, LLM-related items cannot be Must Have for V1 ship. They should be a separate tier — "Must Have IF LLM is included" or moved to Should Have.

**Recommendation:** Rename 11.2 to "Must Have — LLM Features (when LLM is enabled)" or restructure as: "If LLM is present, these features must work. If LLM is absent, the application must function fully without them."

---

### 12. Risk Assessment — PASS

Honest and actionable.

**Missing risk: First-run complexity.**

The current first-run experience requires: (1) install binary, (2) run `agent-trace init`, (3) optionally run `agent-trace model download`, (4) run `agent-trace open`. If the user skips step 3, they get degraded functionality with a warning message. This multi-step setup is a usability risk for adoption.

**Recommendation:** Add a risk entry: "First-run friction — Likelihood: Medium, Impact: Medium. Mitigation: `agent-trace open <path>` should auto-init if no store exists, reducing the required steps to one command."

**Missing risk: TOML manifest merge conflicts.**

If an external tool (git, backup software) modifies the manifest, or if two humans manually edit it, the TOML could become structurally invalid. This is more likely than it sounds — if the store is inside a git repo, `git checkout` could overwrite the manifest.

**Recommendation:** Add a risk entry and a mitigation: "`agent-trace repair` can rebuild from filesystem state."

---

### 13. Testing Strategy — MODERATE

Tests are listed but not connected to requirements. See Section 11 feedback above.

**Gap: No performance test plan.** Section 10 has specific performance targets (poll cycle < 10ms for 500 files, manifest parse < 100ms) but the testing strategy doesn't mention performance testing.

**Recommendation:** Add "13.5 Performance Tests" with scenarios for polling, manifest parsing, and snapshot creation at target scale (500 documents).

**Gap: No end-to-end user scenario tests.** The testing strategy covers unit and integration tests but not user workflows. Example: "User initializes a store, adds 3 documents, an agent modifies one, user views the diff, restores a previous version." These end-to-end scenarios catch issues that unit tests miss.

---

### 14. Future Considerations — PASS

Well-prioritized. The `libgit2` integration idea is smart for long-term.

---

### 15. Open Questions — GOOD

These are the right questions. My recommendations:

1. **Model size:** 3B default is correct. Users who want better can upgrade.
2. **Conflict resolution:** Last-write-wins is fine for V1. Both versions are captured, so no data is lost.
3. **Agent lock:** File-based is fine for V1. It's simple and agents can implement it trivially.
4. **Poll interval:** 1 second is correct. Sub-second adds CPU load for negligible UX benefit.
5. **Document size limits:** Warn at 1MB, hard-reject at 10MB. A 10MB markdown file will make diffing and snapshotting expensive.

---

## Cross-Cutting Issues

### Issue A: Missing User Journey

The PRD describes features in isolation but never walks through a complete user journey from start to finish. Adding two concrete scenarios would dramatically improve implementer understanding:

**Scenario 1: Solo developer, first use**
1. User installs `agent-trace` binary
2. Has an existing project folder with 5 markdown planning docs
3. Runs `agent-trace open ./my-project`
4. `agent-trace` detects no `.agent-trace/` — asks "Initialize a new store? [y/N]"
5. User confirms. Store initialized. 5 files detected, registered as `scratch`.
6. TUI launches. Tree shows 5 files with `[S]` type indicators.
7. User types `reclassify prd.md plan` in the chat bar. Tree updates to show `[P]`.
8. User starts Claude Code in another terminal, which creates `tasks/auth.md`.
9. Next poll cycle: `agent-trace` detects the new file, shows it in green in the tree, adds a changelog entry.
10. User types `info tasks/auth.md` to see what was created.

**Scenario 2: Returning user with agent session**
1. User has an existing store with 20 documents.
2. Runs `agent-trace open ./my-project --agent=claude-code`
3. TUI launches, shows all documents, last 50 changelog entries.
4. Claude Code modifies 3 files over the next 10 minutes.
5. Each modification appears in the changelog panel within 1 second (poll cycle).
6. Changelog entries show "Agent `claude-code` modified ..." with LLM-generated summaries (or diff stats if no LLM).
7. User types `diff tasks/api.md` to see what changed in the latest version.
8. User types `restore tasks/api.md v2` to revert one file.
9. User presses `Ctrl+C` to exit. All state is saved.

### Issue B: No Error Message Catalog

The PRD describes many error conditions (corrupted manifest, missing snapshots, stale locks, LLM failures) but doesn't specify what the user sees. A brief error message catalog would help implementers produce consistent, helpful messages:

| Condition | Message |
|-----------|---------|
| Store not initialized | "Not a agent-trace store. Run `agent-trace init <path>` to create one." |
| Corrupted manifest | "Manifest is corrupted. Run `agent-trace repair` to rebuild from filesystem." |
| LLM not configured | "LLM not configured. NL commands disabled. Run `agent-trace model download` to set up." |
| Concurrent instance | "Another agent-trace instance is running (PID: 12345). Opening in read-only mode." |
| File not tracked | "`notes.md` is not tracked. Run `agent-trace add scratch notes.md` to track it." |
| Snapshot missing | "Warning: Snapshot for `prd.md` v2 is missing. Version may be unrecoverable." |

### Issue C: Relationship Between `agent-trace init` and `agent-trace open`

The PRD has both commands but the boundary is fuzzy:

- `agent-trace init /path` creates a store.
- `agent-trace open /path` launches the TUI.
- If the user runs `agent-trace open` on a non-store directory, what happens? Error? Auto-init?

The "Missing risk" in Section 12 feedback suggested `agent-trace open` should auto-init. The PRD should take a position on this.

**Recommendation:** `agent-trace open /path` on a non-store directory should prompt: "No store found at /path. Initialize one? [y/N]". This collapses the two-step init+open into one command for the common case.

### Issue D: No Specification for the `agent-trace status` Display Format

`agent-trace status` is referenced in both CLI and TUI contexts, but the output format is never shown. Implementers need to know what it looks like:

```
Store: my-project (12 documents)
Path:  /home/user/my-project

Modified:
  ~ prd.md                    (plan)    v3   modified 2 minutes ago
  ~ tasks/api.md              (plan)    v2   modified 15 minutes ago

Untracked:
  ? notes.md
  ? drafts/brainstorm.md

Deleted:
  x old-spec.md               (plan)    was v4   deleted 1 hour ago
```

### Issue E: Tags Are Under-Specified

The manifest includes a `tags` field but the PRD never describes how tags are used, set, or queried. Can you filter by tag? Add tags via CLI? Are tags free-form strings?

**Recommendation:** Either remove tags from V1 (simplify) or add a brief section: "Tags are free-form strings. `agent-trace tag <file> <tag>` adds a tag. `agent-trace ls --tag=<tag>` filters by tag. Tags have no system-level meaning; they are for user organization."

---

## Consolidated Recommendations (Priority Order)

### Must Fix Before Implementation Handoff

1. **Fix the version history TOML schema** — the example uses manifest-nested syntax but should be standalone file syntax (Section 4.6)
2. **Fix doc_id format** — pick UUID or sequential, not both (Section 4.4)
3. **Specify batch write behavior per poll cycle** — one manifest write per cycle, not per file (Section 4.4)
4. **Clarify LLM "Must Have" vs optional** — resolve the contradiction in Section 11
5. **Add LLM request channel from Change Processor** — missing from threading model (Section 7.3)
6. **Specify non-blocking LLM summarization** — summary fills in after creation, not blocking (Section 7.3)

### Should Fix for Quality

7. Add two user journey scenarios (Issue A)
8. Add `agent-trace status` output format (Issue D)
9. Specify structured command parsing order in chat bar (Section 5.3.2)
10. Specify panel width proportions and overflow behavior (Section 6)
11. Specify `agent-trace open` behavior on non-store directory (Issue C)
12. Decide on tags: include or cut from V1 (Issue E)
13. Add acceptance test matrix or link DoD items to test scenarios (Section 11)

### Nice to Have

14. Add error message catalog (Issue B)
15. Add exit code specification (Section 8)
16. Add performance test plan (Section 13)
17. Specify help overlay contents (Section 6)
18. Add current-workaround context to problem statement (Section 2)
