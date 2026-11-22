## Description

Provide a clear and concise description of the changes proposed in this pull request.

---

## Type of Change

- [ ] Bug fix (non-breaking change fixing an issue)
- [ ] New feature (non-breaking change adding new functionality)
- [ ] Breaking change (fix or feature causing existing functionality to break)
- [ ] Performance optimization
- [ ] Documentation update
- [ ] Release / Version bump

---

## Pre-Merge Engineering Verification

Run these locally before opening or merging:

- [ ] Code formatted: `cargo fmt --all --check`
- [ ] Linter passing: `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] Full test suite passing: `cargo test --workspace` (all 99+ tests pass)
- [ ] Project builds cleanly: `cargo check --workspace --all-targets`
- [ ] Zero emoji policy: No emoji characters introduced in code, docs, or CLI output

---

## Multi-Platform Release Checklist

*Complete this section when preparing a release or bumping versions merged into `main`.*

### 1. Version Synchronization
- [ ] Workspace root `Cargo.toml`: `version` bumped under `[workspace.package]`
- [ ] Core crate `core/Cargo.toml`: version constraints verified
- [ ] CLI crate `cli/Cargo.toml`: dependency `nodewipe-core` version matches
- [ ] NPM wrapper `npm-package/package.json`: `version` updated

### 2. Package Managers & Manifests
- [ ] **crates.io**: Crates ready for publish (`nodewipe-core` -> `nodewipe`)
- [ ] **Homebrew Tap**: `packaging/homebrew/Formula/nodewipe.rb` updated for `KADHIRAVANEG/homebrew-nodewipe`
- [ ] **Scoop Bucket**: `packaging/scoop/nodewipe.json` updated for `KADHIRAVANEG/scoop-nodewipe`
- [ ] **Windows Winget**: Manifest in `packaging/winget/KADHIRAVANEG.nodewipe.yaml` updated
- [ ] **Arch Linux AUR**: `packaging/aur/PKGBUILD` and `.SRCINFO` updated
- [ ] **NPM Distribution**: `@joker53/nodewipe` wrapper verified

### 3. Documentation & Verification
- [ ] `README.md` badges, installation commands, and version tags updated
- [ ] Release notes / changelog entry prepared

---

> [!NOTE]
> Merging a version bump into `main` triggers the Continuous Delivery (`cd.yml`) pipeline. Once automated tests pass, the pipeline will wait for repository maintainer approval before publishing to crates.io and triggering the full multi-platform release matrix.
