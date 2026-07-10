#!/usr/bin/env bash
# nodewipe installer
# Usage: curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/scripts/install.sh | bash
set -euo pipefail

REPO="KADHIRAVANEG/nodewipe"
API_LATEST="https://api.github.com/repos/${REPO}/releases/latest"

info()  { printf "\033[1;34m==>\033[0m %s\n" "$1"; }
warn()  { printf "\033[1;33mwarn:\033[0m %s\n" "$1" >&2; }
error() { printf "\033[1;31merror:\033[0m %s\n" "$1" >&2; exit 1; }

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *) error "Unsupported OS: $os. Windows users: download the .exe/.msi directly from the GitHub Releases page." ;;
  esac

  case "$arch" in
    x86_64|amd64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *) error "Unsupported architecture: $arch" ;;
  esac

  if [ "$PLATFORM" = "macos" ] && [ "$ARCH" = "aarch64" ]; then
    CLI_ASSET="nodewipe-macos-aarch64"
  elif [ "$PLATFORM" = "macos" ]; then
    CLI_ASSET="nodewipe-macos-x86_64"
  else
    CLI_ASSET="nodewipe-linux-x86_64"
  fi
}

# Finds the actual download URL for an asset matching a pattern (e.g. "*.AppImage"),
# since Tauri bundle filenames include the version number and aren't fixed.
find_asset_url() {
  local pattern="$1"
  curl -fsSL "$API_LATEST" \
    | grep '"browser_download_url"' \
    | grep -i -E "$pattern" \
    | head -n1 \
    | cut -d '"' -f4
}

# ---------- CLI install ----------

choose_install_dir() {
  if [ -w /usr/local/bin ] 2>/dev/null; then
    INSTALL_DIR="/usr/local/bin"
    return
  fi
  if command -v sudo >/dev/null 2>&1; then
    echo
    read -rp "Install system-wide to /usr/local/bin? Needs sudo. [Y/n]: " sw
    if [ "${sw:-Y}" != "n" ] && [ "${sw:-Y}" != "N" ]; then
      INSTALL_DIR="/usr/local/bin"
      USE_SUDO=1
      return
    fi
  fi
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
}

install_binary() {
  local url="$1" dest="$2"
  if [ "${USE_SUDO:-0}" = "1" ]; then
    curl -fsSL "$url" -o /tmp/nodewipe-download
    sudo install -m 755 /tmp/nodewipe-download "$dest"
    rm -f /tmp/nodewipe-download
  else
    curl -fsSL "$url" -o "$dest"
    chmod +x "$dest"
  fi
}

ensure_on_path() {
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) return ;;
  esac

  local rc=""
  case "${SHELL:-}" in
    */fish) rc="$HOME/.config/fish/config.fish" ;;
    */zsh)  rc="$HOME/.zshrc" ;;
    *)      rc="$HOME/.bashrc" ;;
  esac

  local line
  if [[ "$rc" == *fish ]]; then
    line="fish_add_path $INSTALL_DIR"
  else
    line="export PATH=\"$INSTALL_DIR:\$PATH\""
  fi

  if [ -f "$rc" ] && grep -qF "$INSTALL_DIR" "$rc" 2>/dev/null; then
    return # already added previously
  fi

  echo "$line" >> "$rc"
  info "Added $INSTALL_DIR to PATH in $rc (restart your shell, or run: source $rc)"
}

install_cli() {
  info "Detected platform: ${PLATFORM}/${ARCH}"
  choose_install_dir

  local url dest
  url="$(find_asset_url "$CLI_ASSET")"
  [ -n "$url" ] || error "Could not find a release asset for $CLI_ASSET. Has v0.1.0+ been published yet?"
  dest="$INSTALL_DIR/nodewipe"

  info "Downloading CLI from $url"
  install_binary "$url" "$dest"
  info "Installed CLI to $dest"

  ensure_on_path
}

# ---------- GUI install ----------

install_gui_linux() {
  info "Fetching GUI AppImage..."
  local url dest
  url="$(find_asset_url '\.AppImage')"
  [ -n "$url" ] || { warn "No AppImage found in the latest release yet."; return; }

  dest="$INSTALL_DIR/nodewipe-gui.AppImage"
  curl -fsSL "$url" -o "$dest"
  chmod +x "$dest"
  info "Installed GUI to $dest"

  # Desktop launcher entry so it shows up in the app menu like a normal app.
  local desktop_dir="$HOME/.local/share/applications"
  mkdir -p "$desktop_dir"
  cat > "$desktop_dir/nodewipe.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=nodewipe
Comment=Find and reclaim disk space from stray dev artifacts
Exec=$dest
Terminal=false
Categories=Utility;Development;
EOF
  info "Added application menu entry (nodewipe)"
}

install_gui_macos() {
  info "Fetching GUI .dmg..."
  local url tmp_dmg mount_point
  url="$(find_asset_url '\.dmg')"
  [ -n "$url" ] || { warn "No .dmg found in the latest release yet."; return; }

  tmp_dmg="/tmp/nodewipe.dmg"
  curl -fsSL "$url" -o "$tmp_dmg"

  mount_point="$(hdiutil attach "$tmp_dmg" -nobrowse | tail -1 | awk '{print $NF}')"
  local app_path
  app_path="$(find "$mount_point" -maxdepth 1 -name "*.app" | head -n1)"
  [ -n "$app_path" ] || { warn "Couldn't find .app inside the dmg."; hdiutil detach "$mount_point" >/dev/null; return; }

  cp -R "$app_path" /Applications/
  hdiutil detach "$mount_point" >/dev/null
  rm -f "$tmp_dmg"
  info "Installed GUI to /Applications/$(basename "$app_path")"
}

install_gui() {
  if [ "$PLATFORM" = "linux" ]; then
    install_gui_linux
  else
    install_gui_macos
  fi
}

main() {
  detect_platform

  echo "nodewipe installer"
  echo "-------------------"
  echo "1) CLI only (terminal tool, smallest download)"
  echo "2) CLI + GUI (desktop app, installs both)"
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
