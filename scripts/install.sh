#!/usr/bin/env bash
# nodewipe installer
# Usage: curl -fsSL https://raw.githubusercontent.com/<you>/nodewipe/main/scripts/install.sh | bash
set -euo pipefail

REPO="your-username/nodewipe"   # TODO: update once pushed to GitHub
INSTALL_DIR="${NODEWIPE_INSTALL_DIR:-$HOME/.local/bin}"

info()  { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
error() { printf "\033[1;31merror:\033[0m %s\n" "$1" >&2; exit 1; }

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *) error "Unsupported OS: $os. Windows users: download the .exe directly from the GitHub Releases page." ;;
  esac

  case "$arch" in
    x86_64|amd64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *) error "Unsupported architecture: $arch" ;;
  esac

  # There's no macOS aarch64 build target listed for some older releases;
  # fall back to x86_64 (runs fine under Rosetta) if that happens.
  if [ "$PLATFORM" = "macos" ] && [ "$ARCH" = "aarch64" ]; then
    ASSET="nodewipe-macos-aarch64"
  elif [ "$PLATFORM" = "macos" ]; then
    ASSET="nodewipe-macos-x86_64"
  else
    ASSET="nodewipe-linux-x86_64"
  fi
}

latest_release_url() {
  local asset="$1"
  echo "https://github.com/${REPO}/releases/latest/download/${asset}"
}

install_cli() {
  info "Detected platform: ${PLATFORM}/${ARCH}"
  mkdir -p "$INSTALL_DIR"

  local url dest
  url="$(latest_release_url "$ASSET")"
  dest="$INSTALL_DIR/nodewipe"

  info "Downloading CLI from $url"
  curl -fsSL "$url" -o "$dest" || error "Download failed. Has a release been published yet?"
  chmod +x "$dest"

  info "Installed CLI to $dest"

  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
      echo
      echo "Add this to your shell config to use 'nodewipe' from anywhere:"
      echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
      ;;
  esac
}

install_gui() {
  info "GUI installers are platform-specific packages (.AppImage / .dmg / .msi)."
  info "Fetching the latest GUI release list..."
  local page="https://github.com/${REPO}/releases/latest"
  echo "Open this page and download the installer matching your OS:"
  echo "  $page"
}

main() {
  detect_platform

  echo "nodewipe installer"
  echo "-------------------"
  echo "1) CLI only (terminal tool, smallest download)"
  echo "2) CLI + GUI (desktop app installer)"
  read -rp "Choose [1/2]: " choice

  case "$choice" in
    1) install_cli ;;
    2)
      install_cli
      install_gui
      ;;
    *) error "Invalid choice: $choice" ;;
  esac

  echo
  info "Done. Run 'nodewipe' to start the interactive scanner."
}

main "$@"
