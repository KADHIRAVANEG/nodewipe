# Contributing to nodewipe

Thank you for your interest in contributing to **nodewipe**!

## How to Contribute

1. **Fork the repository** and clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/nodewipe.git
   cd nodewipe
   ```
2. **Create a feature** branch:
  ```bash
  git checkout -b feature/amazing-feature
  ```
3. **Make your changes and ensure the code** builds and tests pass:
  ```bash
  cargo test
  cargo build --release
  ```
4. **Commit your changes** using Conventional Commits:
  ```bash
  git commit -m "feat: add support for new artifact type"
  ```
5. **Push to your fork and submit a Pull Request**.


## Development Setup

- Core + CLI: Rust (latest stable)
- GUI: Rust + Node.js + Tauri prerequisites
- Format code: `cargo fmt --all`
- Lint code: `cargo clippy --workspace --all-targets -- -D warnings`
- Run test suite: `cargo test --workspace`

## Pull Request Guidelines

- Keep PRs focused on a single change
- Add tests for new features and bug fixes
- Run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` before submitting
- Update documentation when necessary
- Ensure the code follows Rust idioms
