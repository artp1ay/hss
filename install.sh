#!/usr/bin/env bash
set -euo pipefail

REPO="artp1ay/hss"
BIN="hss"
INSTALL_DIR="${HOME}/.local/bin"

# ── Platform detection ────────────────────────────────────────────────────────
case "$(uname -s)" in
  Linux)  OS="linux"  ;;
  Darwin) OS="macos"  ;;
  *)      echo "error: unsupported OS — $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64)        ARCH="x86_64"  ;;
  arm64|aarch64) ARCH="aarch64" ;;
  *)             echo "error: unsupported architecture — $(uname -m)" >&2; exit 1 ;;
esac

ASSET="${BIN}-${OS}-${ARCH}"

# ── Fetch latest release info ─────────────────────────────────────────────────
echo "Fetching latest release..."
RELEASE=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")

TAG=$(printf '%s' "$RELEASE" | grep '"tag_name"' | head -1 | cut -d'"' -f4)
URL=$(printf '%s' "$RELEASE" | grep "browser_download_url" | grep "\"${ASSET}\"" | head -1 | cut -d'"' -f4)

if [ -z "$URL" ]; then
  echo "error: no binary for '${ASSET}' in release ${TAG}" >&2
  echo "       download manually: https://github.com/${REPO}/releases" >&2
  exit 1
fi

# ── Download to temp, then move atomically ────────────────────────────────────
mkdir -p "$INSTALL_DIR"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

echo "Installing ${BIN} ${TAG} → ${INSTALL_DIR}/${BIN}"
curl -fL --progress-bar "$URL" -o "$TMP"
chmod +x "$TMP"
mv "$TMP" "${INSTALL_DIR}/${BIN}"
trap - EXIT

echo "Done. Run: hss --version"

# ── PATH hint (only shown when hss is not yet reachable) ─────────────────────
if ! command -v "$BIN" &>/dev/null 2>&1; then
  echo ""
  echo "~/.local/bin is not in your PATH. Add it:"
  echo "  # bash"
  echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
  echo "  # zsh"
  echo "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc  && source ~/.zshrc"
fi
