#!/usr/bin/env sh
# Upload dist/*.tar.gz(.sha256) to this project's GitLab generic package registry, as
# package `fastpilot`, version from Cargo.toml. In CI it uses CI_JOB_TOKEN; elsewhere set
# GITLAB_TOKEN (api scope). Install script, Homebrew formula and npm package download from there.
set -eu
cd "$(dirname "$0")/.."
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
api="${CI_API_V4_URL:-https://gitlab.com/api/v4}"
project="${CI_PROJECT_ID:-88423537}"
if [ -n "${CI_JOB_TOKEN:-}" ]; then
  header="JOB-TOKEN: $CI_JOB_TOKEN"
elif [ -n "${GITLAB_TOKEN:-}" ]; then
  header="PRIVATE-TOKEN: $GITLAB_TOKEN"
else
  echo "set GITLAB_TOKEN (a token with api scope) to publish" >&2
  exit 2
fi
found=0
for f in dist/fastpilot-"$version"-*.tar.gz dist/fastpilot-"$version"-*.tar.gz.sha256; do
  [ -f "$f" ] || continue
  found=1
  url="$api/projects/$project/packages/generic/fastpilot/$version/$(basename "$f")"
  curl --fail --silent --show-error --header "$header" --upload-file "$f" "$url" >/dev/null
  echo "uploaded $(basename "$f")"
done
[ "$found" = 1 ] || { echo "nothing to publish in dist/ for $version (run scripts/package.sh)" >&2; exit 1; }
