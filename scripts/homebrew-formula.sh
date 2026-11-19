#!/usr/bin/env sh
# Print the Homebrew formula for a released version (checksums from dist/ when present, else
# from the package registry), e.g. into a tap repository:
#   scripts/homebrew-formula.sh 0.1.0 > ../homebrew-fastpilot/Formula/fastpilot.rb
set -eu
cd "$(dirname "$0")/.."
version="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
base="${FASTPILOT_DOWNLOAD_BASE:-https://github.com/autobot-lab/fastpilot/releases/download}"

sum() {
  f="fastpilot-$version-$1.tar.gz.sha256"
  if [ -f "dist/$f" ]; then s="$(cut -d' ' -f1 < "dist/$f")"; else s="$(curl -fsSL "$base/v$version/$f" | cut -d' ' -f1)" || s=""; fi
  case "$s" in
    [0-9a-f][0-9a-f][0-9a-f][0-9a-f]*) echo "$s" ;;
    *) echo "no checksum for $1 $version (dist/$f or $base/v$version/$f): package and publish it first" >&2; exit 1 ;;
  esac
}
url() { echo "$base/v$version/fastpilot-$version-$1.tar.gz"; }

# Every checksum first: a missing platform stops here, before any output.
arm="$(sum aarch64-apple-darwin)"
intel="$(sum x86_64-apple-darwin)"
linux="$(sum x86_64-unknown-linux-gnu)"

cat <<RUBY
# typed: strict
# frozen_string_literal: true

# fastpilot: AI-driven QA for web, Android and iOS (prebuilt binaries from GitHub releases).
class Fastpilot < Formula
  desc "AI-driven QA for web, Android and iOS: MCP server and shell CLI"
  homepage "https://github.com/autobot-lab/fastpilot"
  version "$version"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "$(url aarch64-apple-darwin)"
      sha256 "$arm"
    end
    on_intel do
      url "$(url x86_64-apple-darwin)"
      sha256 "$intel"
    end
  end

  on_linux do
    on_intel do
      url "$(url x86_64-unknown-linux-gnu)"
      sha256 "$linux"
    end
  end

  def install
    bin.install "fastpilot"
  end

  def caveats
    <<~EOS
      Register it with Claude Code:  claude mcp add fastpilot -- fastpilot mcp
      Web needs Chrome or Chromium; Android needs adb; iOS needs Xcode and a runner built
      from a checkout (ios-runner/build.sh).
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/fastpilot --version")
  end
end
RUBY
