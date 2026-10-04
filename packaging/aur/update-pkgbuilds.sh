#!/usr/bin/env bash
# Rewrite pkgver, pkgrel, and sha256sums for both AUR packages, then refresh .SRCINFO.
# Usage: update-pkgbuilds.sh <version>
set -euo pipefail

version="${1:?usage: update-pkgbuilds.sh <version>}"
version="${version#v}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  printf 'version must look like 0.1.1, got %s\n' "$version" >&2
  exit 1
fi

aur=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch() {
  local url="$1" dest="$2" attempt
  for attempt in 1 2 3 4 5 6; do
    if curl -fsSL --retry 3 --retry-delay 2 -o "$dest" "$url"; then
      return 0
    fi
    printf 'download failed (%s), attempt %s\n' "$url" "$attempt" >&2
    sleep $((attempt * 5))
  done
  printf 'failed to download %s\n' "$url" >&2
  return 1
}

bin_name="retromarchy-${version}-x86_64-linux.tar.gz"
fetch "https://github.com/dannyowelch/retromarchy/releases/download/v${version}/SHA256SUMS" "$tmp/SHA256SUMS"
bin_sum=$(grep -E "^[0-9a-f]{64}  ${bin_name}$" "$tmp/SHA256SUMS" | awk '{print $1}')
if [[ ! "$bin_sum" =~ ^[0-9a-f]{64}$ ]]; then
  printf 'SHA256SUMS has no entry for %s\n' "$bin_name" >&2
  exit 1
fi

fetch "https://github.com/dannyowelch/retromarchy/archive/refs/tags/v${version}.tar.gz" "$tmp/source.tar.gz"
src_sum=$(sha256sum "$tmp/source.tar.gz" | awk '{print $1}')

set_pkgver() {
  local file="$1" sum="$2"
  grep -q '^pkgver=' "$file"
  grep -q '^pkgrel=' "$file"
  grep -q '^sha256sums=' "$file"
  sed -i "s/^pkgver=.*/pkgver=${version}/" "$file"
  sed -i 's/^pkgrel=.*/pkgrel=1/' "$file"
  sed -i "s/^sha256sums=.*/sha256sums=('${sum}')/" "$file"
}

set_pkgver "$aur/retromarchy-bin/PKGBUILD" "$bin_sum"
set_pkgver "$aur/retromarchy/PKGBUILD" "$src_sum"

print_srcinfo() {
  local dir="$1" out work
  if [[ "$(id -u)" -eq 0 ]]; then
    if ! id builder >/dev/null 2>&1; then
      useradd --create-home --user-group builder
    fi
    work=$(mktemp -d)
    chmod 755 "$work"
    cp "$dir/PKGBUILD" "$work/PKGBUILD"
    chown builder:builder "$work" "$work/PKGBUILD"
    out=$(runuser -u builder -- bash -c 'cd "$1" && makepkg --printsrcinfo' bash "$work")
    rm -rf "$work"
  else
    out=$(cd "$dir" && makepkg --printsrcinfo)
  fi
  printf '%s\n' "$out" > "$dir/.SRCINFO"
}

print_srcinfo "$aur/retromarchy-bin"
print_srcinfo "$aur/retromarchy"

printf 'retromarchy-bin %s sha256 %s\n' "$version" "$bin_sum"
printf 'retromarchy %s sha256 %s\n' "$version" "$src_sum"
