#!/bin/sh
#
# Install tuls from a GitHub Release binary archive.
#
# The release workflow (release.yml) uploads one archive per target x Rust
# toolchain, named like:
#
#   tuls-<target>-<rust-msrv|rust-stable>.tar.gz
#
# This script detects the host platform, downloads the matching archive
# from the latest release (or a specific version), and installs the binary.
#
# Usage:
#   sh install.sh               install the latest release
#   sh install.sh v0.1.0        install a specific version (tag)
#
# Environment:
#   PREFIX           install directory (default: ~/.local/bin)
#   TULS_RUST_LABEL  archive label to fetch: "rust-msrv" (default) or "rust-stable"
#   TULS_TARGET      override the detected target triple
#
# The default PREFIX (~/.local/bin) matches the ~/.cargo/bin convention and
# needs no privileges. Set PREFIX=/usr/local/bin (or run with sudo) for a
# system-wide install.

set -eu

repo="tuls-rs/tuls"
bin_name="tuls"
label="${TULS_RUST_LABEL:-rust-msrv}"
prefix="${PREFIX:-$HOME/.local/bin}"
version="${1:-latest}"

if command -v curl >/dev/null 2>&1; then
  download() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget -q "$1" -O "$2"; }
else
  echo "install.sh: need curl or wget to download the release archive" >&2
  exit 1
fi

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux)
      case "$arch" in
        x86_64)  echo "x86_64-unknown-linux-gnu" ;;
        aarch64) echo "aarch64-unknown-linux-gnu" ;;
        armv7l|armv7) echo "armv7-unknown-linux-musleabihf" ;;
        *) echo "unsupported Linux architecture: $arch" >&2; exit 1 ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64) echo "x86_64-apple-darwin" ;;
        arm64)  echo "aarch64-apple-darwin" ;;
        *) echo "unsupported macOS architecture: $arch" >&2; exit 1 ;;
      esac
      ;;
    *)
      echo "install.sh: unsupported platform: $os" >&2
      echo "Prebuilt binaries are available for Linux, macOS, and Windows." >&2
      echo "On Windows, download the .zip archive or use: cargo install tuls" >&2
      exit 1
      ;;
  esac
}

target="${TULS_TARGET:-$(detect_target)}"

if [ "$version" = "latest" ]; then
  base_url="https://github.com/$repo/releases/latest/download"
else
  base_url="https://github.com/$repo/releases/download/$version"
fi
archive_url="$base_url/${bin_name}-${target}-${label}.tar.gz"

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/tuls-install.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM

echo "Downloading $archive_url"
download "$archive_url" "$tmp_dir/${bin_name}.tar.gz"

mkdir -p "$tmp_dir/extract"
tar -xzf "$tmp_dir/${bin_name}.tar.gz" -C "$tmp_dir/extract"

# The archive layout varies; locate the binary by name.
bin_path="$(find "$tmp_dir/extract" -type f -name "$bin_name" | head -n 1)"
if [ -z "$bin_path" ]; then
  echo "install.sh: $bin_name binary not found in the archive" >&2
  exit 1
fi

mkdir -p "$prefix"
install -m 0755 "$bin_path" "$prefix/$bin_name"

"$prefix/$bin_name" --version
echo "Installed $bin_name to $prefix/$bin_name"
echo "Ensure $prefix is on your PATH."