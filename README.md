# npkill-rs (working name)

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

What's *not* built yet (next steps, see Roadmap):
- GUI (planned with Tauri, reusing `npkill-core` unchanged).
- `.npkillignore` / exclude-pattern config file.
- Progress bar during long scans (current TUI blocks until the initial scan finishes).

## Building

I don't have network access in this sandbox, so this hasn't been compiled
yet — you'll need to do the first build locally:

```bash
cd npkill-rs
cargo build --release
./target/release/npkill-rs scan
```

If you hit compile errors, paste them back to me and I'll fix them.

## Usage

```bash
# Human-readable scan of the current directory
npkill-rs scan

# Only show node_modules >= 50MB
npkill-rs scan --min-mb 50

# Group results by monorepo/workspace root
npkill-rs scan --grouped

# Scriptable / CI mode
npkill-rs scan --json > report.json

# Delete: safe by default (moves to OS trash), requires --yes for automation
npkill-rs delete ./apps/foo/node_modules --yes

# Archive before deleting
npkill-rs delete ./apps/foo/node_modules --mode archive --yes

# Preview only
npkill-rs delete ./apps/foo/node_modules --dry-run
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

## Roadmap

1. ✅ Core scanning + deletion engine, scriptable CLI
2. ✅ Interactive terminal UI with `ratatui`, multi-select, trash/archive/permanent
3. `.npkillignore` config file + `--exclude` glob support
4. Tauri GUI wrapping `npkill-core` (no engine code duplicated)
5. Packaging: prebuilt binaries via GitHub Actions for macOS/Linux/Windows
6. Benchmark suite vs. npkill on a large monorepo fixture, published in README

## Project layout

```
npkill-rs/
├── core/     # npkill-core: engine, no I/O with the user — reusable by CLI and GUI
└── cli/      # npkill-cli: binary, argument parsing, output formatting
```
