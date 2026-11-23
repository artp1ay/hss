#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: ./scripts/release.sh <version> [release_notes_file_or_string]"
  echo "Example: ./scripts/release.sh 1.0.0 \"Initial public release\""
  echo "         ./scripts/release.sh 1.0.0 @docs/release-notes.md"
  exit 1
}

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  usage
fi

VERSION="${VERSION#v}"
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  echo "Error: version must follow semantic versioning (e.g. 1.0.0)" >&2
  exit 1
fi

TAG="v${VERSION}"
NOTES="${2:-hss ${TAG}}"

if [[ "$NOTES" =~ ^@ ]]; then
  FILE_PATH="${NOTES#@}"
  if [[ ! -f "$FILE_PATH" ]]; then
    echo "Error: notes file not found: $FILE_PATH" >&2
    exit 1
  fi
  NOTES="$(cat "$FILE_PATH")"
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Error: working directory has uncommitted changes. Please commit or stash them first." >&2
  exit 1
fi

if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Error: git tag '$TAG' already exists." >&2
  exit 1
fi

echo "Creating tag $TAG..."
git tag -a --cleanup=verbatim "$TAG" -m "$NOTES"

echo "Pushing tag $TAG to origin..."
git push origin "$TAG"

echo "Release tag $TAG pushed. GitHub Actions will build and publish the release."
