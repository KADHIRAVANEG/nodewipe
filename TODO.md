# nodewipe — Development TODO

> Derived from exhaustive analysis of all 123 issues in `voidcosmos/npkill` (11 open, 112 closed)
> and a full architectural review of `nodewipe` as of `v0.9.0`.

---

## Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Already implemented and tested in `nodewipe` |
| 🟡 | Core logic exists — needs TUI/CLI wiring or polish |
| 🔴 | Not yet implemented — open opportunity |
| 🚫 | Not applicable to Rust tooling (Node.js-specific bugs) |

---

## Tier 1 — HIGH PRIORITY (Direct npkill Community Pain Points)

### 🟡 T1-1: Collapsed Monorepo View in TUI _(npkill #104)_
**Problem**: Monorepos (pnpm workspaces, Turborepo, Nx, Lerna, Cargo) can generate 20–50+ artifact
rows in a flat list. Users reported scrolling for 10 minutes through `~/Library/Caches/Yarn/v6/*`.

**What's already built:**
- `core/src/workspace.rs` → `annotate_workspace_roots()` detects workspace roots for
  `pnpm-workspace.yaml`, `lerna.json`, `turbo.json`, `nx.json`, `go.work`, `Cargo.toml [workspace]`
- `core/src/workspace.rs` → `group_by_workspace()` groups entries by their root
- CLI `--grouped` flag prints grouped table output (non-TUI)

**What needs to be built:**
- [ ] Add `grouped_view: bool` and `collapsed_workspaces: HashSet<PathBuf>` to `App` struct in `cli/src/tui.rs`
- [ ] Add `c` or `Tab` keybinding in the Results stage to toggle grouped view on/off
- [ ] In grouped view, render one summary row per `WorkspaceGroup`:
  `📦 /home/user/my-monorepo   [23 packages]   4.2 GB   ▶ expand`
- [ ] On `Enter` / `Right arrow` → expand group to show all child entries
- [ ] On `Space` in collapsed state → select entire workspace group for bulk delete
- [ ] Show expanded/collapsed indicator `▶` / `▼` in row
- [ ] Add hint to status bar: `c: toggle grouped view | Tab: expand/collapse`
- [ ] Write integration tests for TUI workspace toggle logic

**Files to modify:** `cli/src/tui.rs`, `core/src/workspace.rs`

---

### 🔴 T1-2: Interactive Ignored Folders View _(npkill #220)_
**Problem**: Users want to selectively ignore specific paths (e.g. active projects) and later review
the list of what they've suppressed.

**What's already built:**
- `core/src/ignorefile.rs` → reads `.nodewipeignore` (global `~/.nodewipeignore` + local)
- Scan correctly skips ignored paths

**What needs to be built:**
- [ ] Add `i` keybinding in Results stage to mark current entry as ignored (append to `~/.nodewipeignore`)
- [ ] Add a new `Stage::Ignored` TUI screen accessible via `I` (capital) to list all currently ignored patterns
- [ ] In Ignored screen: `d` removes a pattern from the ignore file, `Esc` goes back
- [ ] Live-reload ignore patterns after modifying the ignore file (re-trigger scan)
- [ ] Add `nodewipe ignore add <PATH>` and `nodewipe ignore list` CLI subcommands
- [ ] Write tests for ignore-add and ignore-list round-trip

**Files to modify:** `cli/src/tui.rs`, `cli/src/cli_def.rs`, `cli/src/main.rs`, `core/src/ignorefile.rs`

---

### 🔴 T1-3: `node_modules/.ignored` Folder Support _(npkill #220 extended)_
**Problem**: pnpm and npm use a special `node_modules/.ignored` directory that is NOT a user-installed
package — it's a hidden internal folder. `npkill` offers no way to view or manage this separately.

**What needs to be built:**
- [ ] Detect `node_modules/.ignored` as a sub-category of `NodeModules` during scan
- [ ] Surface it in TUI with a distinct label `node_modules/.ignored [pnpm internal]`
- [ ] Warn user before deleting (since it may cause pnpm reinstalls)

**Files to modify:** `core/src/scanner.rs`, `core/src/model.rs`

---

## Tier 2 — MEDIUM PRIORITY (Quality & Feature Completeness)

### 🔴 T2-1: Page Up / Page Down / Home / End Navigation _(npkill #66, #53, #100)_
**Problem**: Large result lists (100+ entries) make keyboard navigation slow. Users asked for
page-level navigation since `npkill` v0.3.

**What needs to be built:**
- [ ] Add `PageUp` / `PageDown` keybindings to jump by visible page height (`terminal_rows - 4`)
- [ ] Add `Home` / `End` keybindings to jump to first/last entry
- [ ] Update help overlay to show new keybindings

**Files to modify:** `cli/src/tui.rs`

---

### 🔴 T2-2: Open Folder in File Manager _(npkill #127)_
**Problem**: Users often want to inspect a directory before deleting it. Requested since 2022.

**What needs to be built:**
- [ ] Add `o` keybinding in Results stage to open the highlighted entry's parent directory
  in the system file manager (`xdg-open` / `open` / `explorer`)
- [ ] Add `O` to open the artifact directory itself

**Files to modify:** `cli/src/tui.rs`

---

### 🔴 T2-3: Copy Path to Clipboard _(npkill #126)_
**Problem**: Users want to copy an artifact path for further inspection (e.g. to run `ls -la` on it).

**What needs to be built:**
- [ ] Add `y` keybinding to yank (copy) current entry's path to system clipboard
  using `cli-clipboard` or `arboard` crate
- [ ] Show brief status flash: `✓ Copied to clipboard`

**Files to modify:** `cli/src/tui.rs`, `cli/Cargo.toml`

---

### 🔴 T2-4: Progress Indicator During Deletion _(npkill #58, #82)_
**Problem**: Large deletions (20+ GB) show no feedback while in progress, leaving users uncertain
if the tool is working or frozen.

**What needs to be built:**
- [ ] Show a deletion progress bar during active deletion: `Deleting... 2.1 GB / 8.4 GB ████░░`
- [ ] Run deletion in a background thread and stream progress via `mpsc` channel
- [ ] Update row status per-entry: `Deleting…` → `✓ Deleted (1.2 GB freed)`

**Files to modify:** `cli/src/tui.rs`, `core/src/deleter.rs`

---

### 🔴 T2-5: `--older-than` Age Filter in TUI _(npkill #257, #76, #64)_
**Problem**: Users want to delete only artifacts from long-abandoned projects (e.g. untouched for
3+ months). This already exists in CLI but not wired into TUI filter screen.

**What needs to be built:**
- [ ] Wire `older_than` age filter into TUI's `SelectTypes` filter screen
- [ ] Add a date-range input widget (e.g. `>30d`, `>3m`, `>1y`)
- [ ] Highlight stale entries visually (dim color if `> 90 days old`)

**Files to modify:** `cli/src/tui.rs`

---

### 🔴 T2-6: Select-All / Deselect-All in TUI _(npkill #192, #176)_
**Problem**: Users frequently want to wipe everything found in one sweep, but must press `Space`
on each individual row.

**What needs to be built:**
- [ ] Add `A` keybinding to select/deselect all entries
- [ ] Add `A` variant that selects only currently visible filtered entries
- [ ] Show selection count in status bar: `12 / 47 selected  (8.3 GB)`

**Files to modify:** `cli/src/tui.rs`

---

### 🟡 T2-7: JSON Output for All Commands _(npkill #194)_
**Problem**: Teams want machine-readable output to integrate with scripts, CI dashboards, or
monitoring systems.

**What's already built:** `--json` flag on `scan` command outputs structured JSON.

**What needs to be built:**
- [ ] Ensure `delete`, `stats`, and `restore` subcommands also honour `--json`
- [ ] Document JSON schema in README and man page

**Files to modify:** `cli/src/main.rs`

---

### 🔴 T2-8: `--target` Support for Multiple Artifact Types _(npkill #173)_
**Problem**: Users want to target only specific artifact types in one scan instead of excluding everything else.

**What needs to be built:**
- [ ] Add `--include-types` / `--types` CLI flag as the inverse of `--exclude-types`
- [ ] Update TUI filter screen to offer an "include only" toggle vs "exclude" mode

**Files to modify:** `cli/src/cli_def.rs`, `cli/src/main.rs`, `cli/src/tui.rs`

---

## Tier 3 — NICE TO HAVE (Polish & Additional Ecosystems)

### 🔴 T3-1: Yarn Cache Aware Scan _(npkill #13)_
**Problem**: `~/.cache/yarn/v6` can contain 10–80 GB of cached tarballs that never get cleaned.
npkill reported impossible totals because it misidentified Yarn cache dirs.

- [ ] Add `Yarn Global Cache` as an explicit `ArtifactKind::YarnCache` with `SelfHasAny` marker
- [ ] Warn: "Deleting Yarn cache will slow down future `yarn install` runs"

---

### 🔴 T3-2: Android / Termux Support _(npkill #274)_
**Problem**: `npkill` crashed on Android/Termux with `process.platform === 'android'` check.

- [ ] Verify `nodewipe` compiles and runs correctly under Termux/Android via cross-compilation target
- [ ] Test `aarch64-linux-android` Cargo build target in CI
- [ ] Add to release matrix in CI workflow

---

### 🔴 T3-3: `.npkillrc` / Config File Migration _(npkill #214)_
**Problem**: `npkill` added a `.npkillrc` config file for persistent settings. Users want to
carry their `--exclude-types`, `--root`, and `--min-size` preferences.

**What's already built:** `nodewipe` has `load_config()` reading a config file.

- [ ] Document the config file format and location in README
- [ ] Add `nodewipe config` subcommand to view/edit config interactively
- [ ] Support `~/.config/nodewipe/config.toml` (XDG compliant)

---

### 🔴 T3-4: Text Rendering / Unicode Safety _(npkill #119, #118, #43)_
**Problem**: Unicode/zalgo text in directory paths caused npkill's terminal layout to break.
npkill also had issues with line-overwrite artifacts in the console.

- [ ] Audit all `ratatui` text rendering paths for Unicode normalization
- [ ] Use `unicode-width` crate to correctly compute display widths for all path labels
- [ ] Add test with deep unicode directory paths

---

### 🚫 NOT APPLICABLE (Node.js-Specific Issues — Already Solved by Architecture)

The following closed npkill issues represent structural limitations of building in Node.js/TypeScript
that are **impossible in `nodewipe`** by design:

| npkill Issue | Root Cause | Why nodewipe is immune |
|---|---|---|
| #34, #25 `spawn cut EMFILE/EAGAIN` | Spawned external `cut` process per file | Zero subprocess spawning; pure syscalls |
| #79 `MaxListenersExceededWarning` | EventEmitter object leaks in Node.js | Rust ownership model; no GC/EventEmitter |
| #177 `windowsStrategyManager is not a function` | Dynamic dispatch bug in TypeScript DI | Strongly typed Rust; no runtime type errors |
| #162, #159 `Cannot read properties of undefined` | JavaScript null/undefined dereference | Rust's `Option<T>` enforces null safety |
| #141 `SyntaxError: Unexpected token '.'` | Node.js optional chaining version mismatch | Compiled binary; no runtime parser |
| #17 HTTP request on startup | Update-check ping to external server | Zero network traffic; offline by default |
| #57 Photos/Contacts macOS permissions | Unrestricted `fs.readdir` scope | Explicit boundary dirs excluded from scan |

---

## Implementation Order (Suggested)

```
Phase 1 (Sprint 3A) → T1-1 Collapsed Monorepo TUI View
Phase 2 (Sprint 3B) → T1-2 Interactive Ignored Folders View + T1-3 .ignored subfolder
Phase 3 (Sprint 3C) → T2-6 Select-All + T2-1 Page Navigation + T2-2 Open Folder + T2-3 Copy Path
Phase 4 (Sprint 3D) → T2-4 Deletion Progress Bar + T2-5 Older-Than TUI filter
Phase 5 (Sprint 4)  → T2-7 JSON completeness + T2-8 --include-types + T3-x ecosystem polish
```

---

## Checklist Progress

- Tier 1: 0/3 implemented
- Tier 2: 0/8 implemented (1 partial)
- Tier 3: 0/4 implemented
- Already solved vs npkill: **9 of 11 open issues** + **all structural root causes in 112 closed issues**
