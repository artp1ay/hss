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

# grep exits 1 on no match — would abort under set -e, so use || true
# URL format in JSON: "…/download/vX.Y.Z/hss-linux-x86_64"  (slash before name, quote after)
TAG=$(printf '%s' "$RELEASE" | grep '"tag_name"'  | head -1 | cut -d'"' -f4 || true)
URL=$(printf '%s' "$RELEASE" | grep "/${ASSET}\"" | head -1 | cut -d'"' -f4 || true)

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

# ── PATH hint ────────────────────────────────────────────────────────────────
if ! command -v "$BIN" &>/dev/null 2>&1; then
  echo ""
  echo "~/.local/bin is not in your PATH. Add it:"
  echo "  # bash:  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc && source ~/.bashrc"
  echo "  # zsh:   echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc  && source ~/.zshrc"
fi

# ── Optional fzf alias ────────────────────────────────────────────────────────
# When piped through curl|bash, stdin is the script — read from /dev/tty instead.
offer_alias() {
  local exe="${INSTALL_DIR}/${BIN}"
  local alias_line="alias ${BIN}s='${exe} --fzf'"

  # Detect current shell's rc file
  local shell_name rc_file
  shell_name=$(basename "${SHELL:-bash}")
  case "$shell_name" in
    bash) rc_file="${HOME}/.bashrc"  ;;
    zsh)  rc_file="${HOME}/.zshrc"   ;;
    fish) rc_file="${HOME}/.config/fish/config.fish" ;;
    *)
      echo ""
      echo "Unknown shell '$shell_name'. Add alias manually:"
      echo "  $alias_line"
      return
      ;;
  esac

  echo ""
  printf "Add alias for fzf mode  (%s)? [y/N] " "$alias_line"

  local answer=""
  # /dev/tty is the real terminal even when stdin is a pipe
  if ! read -r answer < /dev/tty 2>/dev/null; then
    echo ""
    echo "Non-interactive — skipping alias. Add manually: $alias_line"
    return
  fi

  case "$answer" in
    [yY]|[yY][eE][sS])
      # Don't add if already present (exact line match)
      if grep -qF "$alias_line" "$rc_file" 2>/dev/null; then
        echo "Already in $rc_file — nothing to do."
      else
        printf '\n%s\n' "$alias_line" >> "$rc_file"
        echo "Added to $rc_file"
        echo "Run: source $rc_file"
      fi
      ;;
    *)
      echo "Skipped."
      ;;
  esac
}

offer_alias
