#!/usr/bin/env sh
# Upload dist/*.tar.gz(.sha256) of the version in Cargo.toml to the GitHub release v<version>
# (created when missing). In CI it uses GITHUB_TOKEN; elsewhere a logged-in `gh`.
# The install script, Homebrew formula and npm package download from there.
set -eu
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
repo="${GITHUB_REPOSITORY:-autobot-lab/fastpilot}"
command -v gh >/dev/null 2>&1 || { echo "needs the GitHub CLI (gh)" >&2; exit 2; }
set -- dist/fastpilot-"$version"-*.tar.gz dist/fastpilot-"$version"-*.tar.gz.sha256
[ -f "$1" ] || { echo "nothing to publish in dist/ for $version (run scripts/package.sh)" >&2; exit 1; }
if ! gh release view "v$version" --repo "$repo" >/dev/null 2>&1; then
  gh release create "v$version" --repo "$repo" --title "fastpilot v$version" --notes-file docs/release-notes.md --verify-tag
fi
gh release upload "v$version" --repo "$repo" --clobber "$@"
for f in "$@"; do echo "uploaded $(basename "$f")"; done
