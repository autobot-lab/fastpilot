#!/usr/bin/env sh
# Install fastpilot from its GitLab releases:
#   curl -fsSL https://gitlab.com/autobot-lab/fastpilot/-/raw/main/scripts/install.sh | sh
# Options (environment):
#   FASTPILOT_VERSION=0.1.0           a version instead of the latest release
#   FASTPILOT_INSTALL_DIR=/usr/local/bin   where the binary goes (default ~/.local/bin)
#   FASTPILOT_DOWNLOAD_BASE=...       a mirror of the package registry
# The archive's SHA-256 is checked before anything is installed.
set -eu

project="https://gitlab.com/api/v4/projects/88423537"
base="${FASTPILOT_DOWNLOAD_BASE:-$project/packages/generic/fastpilot}"
dir="${FASTPILOT_INSTALL_DIR:-$HOME/.local/bin}"

fail() { echo "fastpilot install: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "needs $1"; }
need curl
need tar

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64) target=aarch64-apple-darwin ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  Linux/x86_64 | Linux/amd64) target=x86_64-unknown-linux-gnu ;;
  *) fail "no prebuilt binary for $os/$arch yet; build from source: https://gitlab.com/autobot-lab/fastpilot#build" ;;
esac

version="${FASTPILOT_VERSION:-}"
if [ -z "$version" ]; then
  tag="$(curl -fsSL "$project/releases/permalink/latest" | sed -n 's/.*"tag_name":"\([^"]*\)".*/\1/p')"
  [ -n "$tag" ] || fail "cannot find the latest release (set FASTPILOT_VERSION)"
  version="${tag#v}"
fi

name="fastpilot-$version-$target"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
echo "downloading $name"
curl -fsSL "$base/$version/$name.tar.gz" -o "$tmp/$name.tar.gz" || fail "download failed: $base/$version/$name.tar.gz"
curl -fsSL "$base/$version/$name.tar.gz.sha256" -o "$tmp/sum" || fail "checksum download failed"
want="$(cut -d' ' -f1 < "$tmp/sum")"
if command -v sha256sum >/dev/null 2>&1; then
  got="$(sha256sum "$tmp/$name.tar.gz" | cut -d' ' -f1)"
else
  got="$(shasum -a 256 "$tmp/$name.tar.gz" | cut -d' ' -f1)"
fi
[ "$want" = "$got" ] || fail "checksum mismatch for $name.tar.gz (expected $want, got $got)"

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
mkdir -p "$dir"
install -m 0755 "$tmp/$name/fastpilot" "$dir/fastpilot"
# Downloaded with curl, so no quarantine flag; cleared anyway in case a browser fetched it.
if [ "$os" = Darwin ]; then xattr -d com.apple.quarantine "$dir/fastpilot" 2>/dev/null || true; fi
echo "installed fastpilot $version to $dir/fastpilot"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "add it to PATH:  export PATH=\"$dir:\$PATH\"" ;;
esac
echo "next: claude mcp add fastpilot -- fastpilot mcp     (or try: fastpilot open https://example.com)"
