# nodewipe

[![GitHub stars](https://img.shields.io/github/stars/KADHIRAVANEG/nodewipe?style=flat-square&color=eab308&logo=github)](https://github.com/KADHIRAVANEG/nodewipe)
[![npm downloads](https://img.shields.io/npm/dt/@joker53/nodewipe?style=flat-square&color=22c55e&logo=npm)](https://www.npmjs.com/package/@joker53/nodewipe)
[![License](https://img.shields.io/badge/License-MIT-blue.svg?style=flat-square)](https://github.com/KADHIRAVANEG/nodewipe/blob/main/LICENSE)

> **Star on GitHub**: If `nodewipe` saved you gigabytes of storage, please support the project with a star on [GitHub](https://github.com/KADHIRAVANEG/nodewipe)!

Find and reclaim disk space from stray dev artifacts — not just `node_modules`.

```
npx @joker53/nodewipe
```

Downloads a small prebuilt Rust binary for your platform (no compiling, no
Rust toolchain needed) and runs the interactive scanner.

## What it finds

- `node_modules` (npm / yarn / pnpm)
- Python virtual environments (`venv`, `.venv`)
- Python caches (`__pycache__`, `.pytest_cache`, `.mypy_cache`, `.ruff_cache`)
- Rust build output (`target/`)
- Maven (`target/`) and Gradle (`build/`) output
- Next.js (`.next`) and Turborepo (`.turbo`) caches
- JS bundler output (`dist/`)
- AI/ML model caches (Hugging Face, Ollama, PyTorch, Triton)
- Mobile & native build caches (Xcode DerivedData, SwiftPM, CMake, Zig, Elixir)

Ambiguous names like `target`/`build`/`dist` are only matched when a marker
file confirms the ecosystem (e.g. `target/` next to a `Cargo.toml`), so an
unrelated folder that happens to share the name is never touched.

## Safe by default

- **OS Trash Default**: Deletions move to your OS trash (recoverable) unless you specify `--mode permanent` or `--mode archive`.
- **Decision Intelligence**: Run `nodewipe explain <path>` (or press `e` in the TUI) to inspect why a folder matched, assess risk, and get the exact command needed to rebuild it.
- **Active Project Pinning**: Run `nodewipe pin add <path>` (or press `p` in the TUI) to lock active projects. Pinned paths are protected from deletion.
- **Dry Run Simulation**: `--dry-run` (or press `P` in the TUI) previews deletion impact without touching anything.

## This package is CLI-only

There's also a desktop GUI, but it isn't distributed through npm (a native
GUI bundle is much larger than this tool should ever cost you to try). Get it
via the installer script or by building from source — see the main repo:
https://github.com/KADHIRAVANEG/nodewipe

## Usage

```bash
nodewipe                        # interactive TUI in the current directory
nodewipe /                      # scan from filesystem root
nodewipe explain .              # inspect directory risk, match reason & rebuild recipe
nodewipe pin add ./my-app       # protect active project from accidental deletion
nodewipe pin list               # list all pinned projects
nodewipe scan --json            # scriptable/CI output
nodewipe scan --exclude-types venv,dist
nodewipe delete <path> --yes    # moves to trash by default
nodewipe types                  # list all supported artifact types
```

Full documentation: https://github.com/KADHIRAVANEG/nodewipe
