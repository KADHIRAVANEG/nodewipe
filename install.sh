#!/usr/bin/env bash
# nodewipe installer
# Usage: curl -fsSL https://raw.githubusercontent.com/KADHIRAVANEG/nodewipe/main/install.sh | bash
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
  elif [ "$PLATFORM" = "linux" ] && [ "$ARCH" = "x86_64" ]; then
    CLI_ASSET="nodewipe-linux-x86_64"
  elif [ "$PLATFORM" = "linux" ] && [ "$ARCH" = "aarch64" ]; then
    CLI_ASSET="nodewipe-linux-aarch64"
  else
    error "Prebuilt binary for $PLATFORM/$ARCH is not currently available. Please build from source using 'cargo build --release'."
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

prompt_user() {
  local prompt="$1"
  local default_val="$2"
  local reply=""

  if [ -t 0 ] && [ -t 1 ]; then
    read -rp "$prompt" reply || reply=""
  elif [ -t 1 ] && [ -r /dev/tty ]; then
    read -rp "$prompt" reply < /dev/tty || reply=""
  else
    reply=""
  fi

  if [ -z "$reply" ]; then
    printf "%s" "$default_val"
  else
    printf "%s" "$reply"
  fi
}

# ---------- CLI install ----------

choose_install_dir() {
  if [ "${SYSTEM_INSTALL:-0}" = "1" ]; then
    if [ -w /usr/local/bin ] 2>/dev/null; then
      INSTALL_DIR="/usr/local/bin"
      return
    fi
    if command -v sudo >/dev/null 2>&1; then
      INSTALL_DIR="/usr/local/bin"
      USE_SUDO=1
      return
    fi
  fi
  # Default to user-space ~/.local/bin (requires no root/sudo privileges and enables seamless self-updates)
  INSTALL_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
  mkdir -p "$INSTALL_DIR"
}

install_binary() {
  local url="$1" dest="$2" asset_name="${3:-}"
  local tmp_dir
  tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t 'nodewipe-install')"
  local tmp_bin="$tmp_dir/nodewipe-bin"

  curl -fsSL "$url" -o "$tmp_bin" || { rm -rf "$tmp_dir"; error "Failed to download $url"; }

  # Verify SHA256 checksum if SHA256SUMS is published in the release
  local sums_url="https://github.com/KADHIRAVANEG/nodewipe/releases/latest/download/SHA256SUMS"
  if [ -n "$asset_name" ] && curl -fsSL "$sums_url" -o "$tmp_dir/SHA256SUMS" 2>/dev/null; then
    if grep -F "$asset_name" "$tmp_dir/SHA256SUMS" >/dev/null 2>&1; then
      local expected_sum actual_sum
      expected_sum=$(grep -F "$asset_name" "$tmp_dir/SHA256SUMS" | awk '{print $1}' | head -n1)
      if command -v sha256sum >/dev/null 2>&1; then
        actual_sum=$(sha256sum "$tmp_bin" | awk '{print $1}')
      elif command -v shasum >/dev/null 2>&1; then
        actual_sum=$(shasum -a 256 "$tmp_bin" | awk '{print $1}')
      fi
      if [ -n "$expected_sum" ] && [ -n "$actual_sum" ]; then
        if [ "$expected_sum" != "$actual_sum" ]; then
          rm -rf "$tmp_dir"
          error "SHA-256 integrity verification failed for $asset_name! Expected: $expected_sum, Got: $actual_sum"
        fi
        info "Verified SHA-256 checksum ($actual_sum)"
      fi
    fi
  fi

  if [ "${USE_SUDO:-0}" = "1" ]; then
    sudo install -m 755 "$tmp_bin" "$dest"
  else
    install -m 755 "$tmp_bin" "$dest" 2>/dev/null || { cp "$tmp_bin" "$dest" && chmod +x "$dest"; }
  fi
  rm -rf "$tmp_dir"
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
  install_binary "$url" "$dest" "$CLI_ASSET"
  info "Installed CLI to $dest"

  # Install man page
  if [ -x "$dest" ]; then
    if [ "${USE_SUDO:-0}" = "1" ] && { [ -d /usr/local/share/man/man1 ] || [ -d /usr/share/man/man1 ]; }; then
      local sys_man="/usr/local/share/man/man1"
      sudo mkdir -p "$sys_man"
      "$dest" man 2>/dev/null | sudo tee "$sys_man/nodewipe.1" >/dev/null || true
      if command -v mandb >/dev/null 2>&1; then
        sudo mandb -u >/dev/null 2>&1 || true
      fi
    else
      local user_man="${XDG_DATA_HOME:-$HOME/.local/share}/man/man1"
      mkdir -p "$user_man"
      "$dest" man > "$user_man/nodewipe.1" 2>/dev/null || true
      if command -v mandb >/dev/null 2>&1; then
        mandb -u >/dev/null 2>&1 || true
      fi
    fi
    info "Installed man page (view with 'man nodewipe')"
  fi

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

  local choice=""
  for arg in "$@"; do
    case "$arg" in
      --cli) choice="1" ;;
      --gui|--all) choice="2" ;;
      -y|--yes) choice="1" ;;
      --system) SYSTEM_INSTALL=1 ;;
    esac
  done

  if [ -z "$choice" ]; then
    echo "nodewipe installer"
    echo "-------------------"
    echo "1) CLI only (terminal tool, smallest download)"
    echo "2) CLI + GUI (desktop app, installs both)"
    choice="$(prompt_user "Choose [1/2] (default: 1): " "1")"
  fi

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
