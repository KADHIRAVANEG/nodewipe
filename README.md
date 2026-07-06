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
- **`gui`**: Tauri desktop app (npkill#186) — search/filter, flat and
  collapsible grouped (monorepo) views, sortable columns, select-all,
  colored package-manager badges, a real confirmation modal for permanent
  delete, and toast notifications. Same `nodewipe-core` engine as the CLI.
- **Distribution scaffolding**: GitHub Actions release workflow, an
  `install.sh` that asks CLI-only vs CLI+GUI, and an `npm-package/` shim so
  `npx nodewipe` works like `npx npkill` — see "Installing" below. Not yet
  live: needs a real GitHub repo + first tagged release.

What's *not* built yet (next steps, see Roadmap):
- `.nodewipeignore` / exclude-pattern config file.
- Progress bar during long scans (current TUI/GUI block until the initial scan finishes).
- Custom app icons for GUI bundling (placeholders in place for now).

## Installing (once releases exist)

Three ways to get `nodewipe`, from simplest to most manual:

```bash
# 1. npm (works like `npx npkill` today) — downloads the right native binary,
#    no Rust/Node build step for the end user
npx nodewipe
# or: npm install -g nodewipe

# 2. Shell installer — asks CLI-only vs CLI+GUI
curl -fsSL https://raw.githubusercontent.com/your-username/nodewipe/main/scripts/install.sh | bash

# 3. Manual — grab the binary for your platform from GitHub Releases
```

None of these compile anything locally — `.github/workflows/release.yml` builds
native binaries for Linux/macOS/Windows on every version tag and attaches them
to a GitHub Release; the npm package and `install.sh` just fetch the matching
one. **This only works once a tagged release has actually been pushed and
built** — see "Publishing a release" below. Until then, build from source
(next section).

### Publishing a release

```bash
git tag v0.1.0
git push origin v0.1.0
```
This triggers the GitHub Actions workflow to build and attach binaries.
Before your first real release, update the placeholder `your-username/nodewipe`
repo references in `scripts/install.sh` and `npm-package/scripts/download-binary.js`
to your actual GitHub username, then publish the npm shim once:
```bash
cd npm-package
npm publish
```

## Building from source

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
