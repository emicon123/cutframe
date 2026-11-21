#!/usr/bin/env bash
# Sets up a development environment on Debian 13 / Raspberry Pi OS (arm64 or x64).
# Idempotent: safe to re-run. Uses sudo only for apt.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

APT_PACKAGES=(
  # Toolchain basics
  build-essential curl file pkg-config git
  # Tauri 2 (WebKitGTK shell)
  libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
  # FFmpeg dev headers (rsmpeg) + libclang for bindgen
  libavcodec-dev libavformat-dev libavutil-dev libavfilter-dev libavdevice-dev
  libswscale-dev libswresample-dev clang libclang-dev
  # wgpu (Vulkan) and cpal (ALSA)
  libvulkan-dev mesa-vulkan-drivers vulkan-tools libasound2-dev
)

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }

# --- apt packages -----------------------------------------------------------
missing=()
for pkg in "${APT_PACKAGES[@]}"; do
  if ! dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q 'install ok installed'; then
    missing+=("$pkg")
  fi
done

if ((${#missing[@]})); then
  log "Installing ${#missing[@]} apt package(s): ${missing[*]}"
  if ! sudo -n true 2>/dev/null && [[ ! -t 0 ]]; then
    warn "sudo needs a password but there is no terminal to ask for it."
    warn "Run this script from a real terminal (desktop or SSH), or install manually:"
    warn "  sudo apt-get install -y --no-install-recommends ${missing[*]}"
    exit 1
  fi
  sudo apt-get update
  sudo apt-get install -y --no-install-recommends "${missing[@]}"
else
  log "All apt packages already installed"
fi

# --- Rust (rustup) ----------------------------------------------------------
if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

if ! command -v rustup >/dev/null 2>&1; then
  log "Installing rustup (stable, minimal profile + rustfmt, clippy)"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --profile minimal --default-toolchain stable -c rustfmt -c clippy
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
else
  log "rustup already installed, updating stable"
  rustup update stable --no-self-update
fi

if [[ -f "$REPO_ROOT/rust-toolchain.toml" ]]; then
  log "Installing toolchain pinned in rust-toolchain.toml"
  (cd "$REPO_ROOT" && rustup toolchain install)
fi

# --- Node / pnpm ------------------------------------------------------------
if ! command -v node >/dev/null 2>&1; then
  warn "Node.js not found. Install Node 24 LTS (e.g. from nodesource or nvm) and re-run."
elif ! command -v pnpm >/dev/null 2>&1; then
  log "Enabling pnpm via corepack"
  corepack enable pnpm 2>/dev/null || warn "corepack failed; install pnpm with: npm install -g pnpm"
fi

# --- Summary ----------------------------------------------------------------
log "Environment summary"
for cmd in rustc cargo node pnpm ffmpeg clang; do
  if command -v "$cmd" >/dev/null 2>&1; then
    printf '  %-8s %s\n' "$cmd" "$("$cmd" --version 2>&1 | head -n1)"
  else
    printf '  %-8s MISSING\n' "$cmd"
  fi
done
for pc in webkit2gtk-4.1 libavcodec libavformat vulkan alsa; do
  printf '  %-14s %s\n' "$pc" "$(pkg-config --modversion "$pc" 2>/dev/null || echo MISSING)"
done
if command -v vulkaninfo >/dev/null 2>&1; then
  gpu="$(vulkaninfo --summary 2>/dev/null | awk -F'= ' '/deviceName/{print $2; exit}')"
  printf '  %-14s %s\n' "vulkan GPU" "${gpu:-none detected}"
fi

cat <<'EOF'

Done. Open a new shell (or run: source "$HOME/.cargo/env"), then from the repo root:
  pnpm install && pnpm tauri dev
EOF
