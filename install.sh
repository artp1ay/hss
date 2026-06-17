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

# ── Fetch latest release ──────────────────────────────────────────────────────
echo "Fetching latest release..."
RELEASE=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")

# grep exits 1 on no match, which would abort under set -e — use || true
# URL format: "…/download/vX.Y.Z/hss-linux-x86_64"  (slash before name, quote after)
TAG=$(printf '%s' "$RELEASE" | grep '"tag_name"'          | head -1 | cut -d'"' -f4 || true)
URL=$(printf '%s' "$RELEASE" | grep "/${ASSET}\""         | head -1 | cut -d'"' -f4 || true)

if [ -z "$TAG" ]; then
  echo "error: could not parse GitHub API response" >&2
  printf '%s\n' "$RELEASE" | head -5 >&2
  exit 1
fi

if [ -z "$URL" ]; then
  echo "error: no binary for '${ASSET}' in release ${TAG}" >&2
  echo "       https://github.com/${REPO}/releases" >&2
  exit 1
fi

# ── Download → temp → atomic move ────────────────────────────────────────────
mkdir -p "$INSTALL_DIR"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

echo "Installing ${BIN} ${TAG} → ${INSTALL_DIR}/${BIN}"
curl -fL --progress-bar "$URL" -o "$TMP"
chmod +x "$TMP"
mv "$TMP" "${INSTALL_DIR}/${BIN}"
trap - EXIT

echo "Done.  Run: hss --version"

# ── PATH hint (only if hss not yet reachable) ─────────────────────────────────
if ! command -v "$BIN" &>/dev/null 2>&1; then
  echo ""
  echo "~/.local/bin is not in your PATH. Add it:"
  echo "  # bash:  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
  echo "  # zsh:   echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc  && source ~/.zshrc"
fi
