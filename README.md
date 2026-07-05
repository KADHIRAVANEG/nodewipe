# nodewipe

```
 _   _           _    __        ___            
| \ | | ___   __| | __\ \      / (_)_ __   ___ 
|  \| |/ _ \ / _` |/ _ \ \ /\ / /| | '_ \ / _ \
| |\  | (_) | (_| |  __/\ V  V / | | |_) |  __/
|_| \_|\___/ \__,_|\___| \_/\_/  |_| .__/ \___|
                                   |_|
```

A Rust reimagining of [npkill](https://github.com/voidcosmos/npkill), built to
directly address the issues open on that repo. See the issue-to-fix mapping
below.

## Status: MVP scaffold

What's implemented right now:
- **`core`**: parallel directory scanner, monorepo/workspace grouping,
  package-manager detection, three delete modes (trash / archive / permanent).
- **`cli`**: interactive TUI (default when run in a terminal — `↑/↓` move,
  `space` select, `d` trash, `a` archive, `p` permanent w/ confirmation,
  `r` rescan, `q` quit), plus `scan`/`delete` subcommands with a
  human-readable and a `--json` (headless/scriptable) output mode.
- **`gui`**: Tauri desktop app (npkill#186) — same `nodewipe-core` engine, a
  plain HTML/JS/CSS frontend (no bundler needed), table view with
  checkboxes, and Trash/Archive/Permanent delete buttons.

What's *not* built yet (next steps, see Roadmap):
- `.nodewipeignore` / exclude-pattern config file.
- Progress bar during long scans (current TUI/GUI block until the initial scan finishes).
- Custom app icons for GUI bundling (placeholders in place for now; fine for
  `tauri dev`, worth swapping before a real `tauri build` release).

## Building the CLI

```bash
cd nodewipe
cargo build --release
./target/release/nodewipe scan
```

## Building the GUI

Requires Node.js/npm in addition to Rust, plus Tauri's native dependencies:
- **Linux**: `webkit2gtk-4.1`, `libappindicator-gtk3`, `librsvg`, and build
  tools — see https://v2.tauri.app/start/prerequisites/ for your distro's
  exact package names (they vary, especially on Arch).
- **macOS**: Xcode Command Line Tools (`xcode-select --install`).
- **Windows**: Microsoft C++ Build Tools + WebView2 (usually already present on Win10/11).

```bash
cd gui
npm install
npm run dev      # launches the app in dev mode with hot reload
npm run build    # produces a distributable bundle
```

## Usage (CLI)

```bash
# Interactive TUI (default when run in a terminal)
nodewipe

# Human-readable scan of the current directory
nodewipe scan

# Only show node_modules >= 50MB
nodewipe scan --min-mb 50

# Group results by monorepo/workspace root
nodewipe scan --grouped

# Scriptable / CI mode
nodewipe scan --json > report.json

# Delete: safe by default (moves to OS trash), requires --yes for automation
nodewipe delete ./apps/foo/node_modules --yes

# Archive before deleting
nodewipe delete ./apps/foo/node_modules --mode archive --yes

# Preview only
nodewipe delete ./apps/foo/node_modules --dry-run
```

## Issue-to-fix mapping (voidcosmos/npkill)

| npkill issue | What this project does differently |
|---|---|
| #188 no headless/scriptable mode | `--json` output + `--yes`/`--dry-run` flags + proper exit codes |
| #199 / #191 nested directory bugs | Scanner prunes recursion only at matched `node_modules`; sibling branches are never affected (see comments in `core/src/scanner.rs`) |
| #104 no grouped/collapsed view | `workspace::group_by_workspace` groups by detected monorepo root |
| #172 / #121 slow scan/delete | `rayon`-parallel directory walk and parallel size computation |
| #60 no trash/undo | `DeleteMode::Trash` moves to OS trash instead of unlinking |
| #46 no archive option | `DeleteMode::Archive` tars+gzips before removing |
| #75 pnpm/Windows issues | Package manager detected from lockfile; `.pnpm` treated as part of one deletable unit, not a nested result |
| #186 no desktop app | `gui/` — Tauri app wrapping the same `nodewipe-core` engine |

## Roadmap

1. ✅ Core scanning + deletion engine, scriptable CLI
2. ✅ Interactive terminal UI with `ratatui`, multi-select, trash/archive/permanent
3. ✅ Tauri GUI wrapping `nodewipe-core` (no engine code duplicated)
4. `.nodewipeignore` config file + `--exclude` glob support
5. Custom app icons + packaging: prebuilt binaries/installers via GitHub Actions for macOS/Linux/Windows
6. Benchmark suite vs. npkill on a large monorepo fixture, published in README

## Project layout

```
nodewipe/
├── core/     # nodewipe-core: engine, no I/O with the user — reusable by CLI and GUI
├── cli/      # nodewipe-cli: binary, argument parsing, output formatting, TUI
└── gui/      # nodewipe-gui: Tauri desktop app (src-tauri/ = Rust backend, rest = frontend)
```
