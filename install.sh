#!/usr/bin/env bash
# Install or remove Retromarchy.
#   curl -fsSL https://raw.githubusercontent.com/dannyowelch/retromarchy/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/dannyowelch/retromarchy/main/install.sh | bash -s -- uninstall
set -euo pipefail

repo="dannyowelch/retromarchy"
name="retromarchy"

die() {
  printf 'install.sh: %s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<EOF
Usage: install.sh [install|uninstall]

On Arch Linux, install downloads the latest release package and runs pacman -U.
Elsewhere it installs the release tarball under ~/.local.

  curl -fsSL https://raw.githubusercontent.com/dannyowelch/retromarchy/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/dannyowelch/retromarchy/main/install.sh | bash -s -- uninstall
EOF
}

need_curl() {
  command -v curl >/dev/null 2>&1 || die "curl is required"
}

arch_like() {
  [[ -r /etc/os-release ]] || return 1
  local id like
  id=$(sed -n 's/^ID=//p' /etc/os-release)
  like=$(sed -n 's/^ID_LIKE=//p' /etc/os-release)
  id="${id%%$'\n'*}"
  like="${like%%$'\n'*}"
  id="${id//\"/}"
  like="${like//\"/}"
  [[ "$id" == "arch" || "$like" == *arch* ]]
}

run_root() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    die "sudo is required to install the Arch package"
  fi
}

latest_tag() {
  local body match tag
  body=$(curl -fsSL --retry 3 --retry-delay 2 \
    "https://api.github.com/repos/${repo}/releases/latest")
  match=$(printf '%s\n' "$body" | grep -m 1 -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' || true)
  tag=$(printf '%s\n' "$match" | sed -n 's/.*"\([^"]*\)"$/\1/p')
  if [[ -z "$tag" || "$tag" != v* ]]; then
    die "could not read the latest release tag from GitHub"
  fi
  printf '%s\n' "$tag"
}

fetch() {
  local url="$1"
  local dest="$2"
  curl -fsSL --retry 3 --retry-delay 2 -o "$dest" "$url"
}

check_file() {
  local file="$1"
  local sums="$2"
  local base dir
  base=$(basename "$file")
  dir=$(dirname "$file")
  if ! grep -F -q "  ${base}" "$sums"; then
    die "SHA256SUMS has no entry for ${base}"
  fi
  if ! (cd "$dir" && grep -F "  ${base}" "$sums" | sha256sum -c -); then
    die "checksum mismatch for ${base}"
  fi
}

install_local() {
  local archive="$1"
  local unpack="${workdir}/unpack"
  local bin src prefix desktop icon
  mkdir -p "$unpack"
  tar -xzf "$archive" -C "$unpack"
  bin=$(find "$unpack" -type f -name "$name" -print -quit)
  [[ -n "$bin" ]] || die "tarball has no ${name} binary"
  src=$(dirname "$bin")
  desktop="${src}/${name}.desktop"
  icon="${src}/${name}.png"
  [[ -f "$desktop" ]] || die "tarball is missing ${name}.desktop"
  [[ -f "$icon" ]] || die "tarball is missing ${name}.png"
  prefix="${HOME}/.local"
  install -d \
    "${prefix}/bin" \
    "${prefix}/share/applications" \
    "${prefix}/share/icons/hicolor/256x256/apps"
  install -m 755 "$bin" "${prefix}/bin/${name}"
  awk -v cmd="${prefix}/bin/${name}" '
    index($0, "Exec=") == 1 { print "Exec=" cmd; next }
    { print }
  ' "$desktop" > "${prefix}/share/applications/${name}.desktop"
  install -m 644 "$icon" "${prefix}/share/icons/hicolor/256x256/apps/${name}.png"
  printf 'Installed %s to %s\n' "$name" "${prefix}/bin/${name}"
}

uninstall_local() {
  local prefix="${HOME}/.local"
  local path
  for path in \
    "${prefix}/bin/${name}" \
    "${prefix}/share/applications/${name}.desktop" \
    "${prefix}/share/icons/hicolor/256x256/apps/${name}.png"
  do
    if [[ -e "$path" ]]; then
      rm -f "$path"
      printf 'Removed %s\n' "$path"
    fi
  done
}

install_release() {
  local tag ver base asset sums
  [[ "$(uname -m)" == "x86_64" ]] || die "releases are built for x86_64 (this machine is $(uname -m))"
  [[ -n "${HOME:-}" ]] || die "HOME is not set"
  need_curl
  tag=$(latest_tag)
  ver="${tag#v}"
  base="https://github.com/${repo}/releases/download/${tag}"
  sums="${workdir}/SHA256SUMS"
  fetch "${base}/SHA256SUMS" "$sums"
  if arch_like && command -v pacman >/dev/null 2>&1; then
    asset="${workdir}/${name}-${ver}-1-x86_64.pkg.tar.zst"
    fetch "${base}/${name}-${ver}-1-x86_64.pkg.tar.zst" "$asset"
    check_file "$asset" "$sums"
    run_root pacman -U --noconfirm "$asset"
    printf 'Installed %s %s with pacman\n' "$name" "$ver"
  else
    asset="${workdir}/${name}-${ver}-x86_64-linux.tar.gz"
    fetch "${base}/${name}-${ver}-x86_64-linux.tar.gz" "$asset"
    check_file "$asset" "$sums"
    install_local "$asset"
  fi
}

uninstall_release() {
  [[ -n "${HOME:-}" ]] || die "HOME is not set"
  if arch_like && command -v pacman >/dev/null 2>&1; then
    if pacman -Q "$name" >/dev/null 2>&1; then
      run_root pacman -Rns --noconfirm "$name"
      printf 'Removed pacman package %s\n' "$name"
    fi
  fi
  uninstall_local
}

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

case "${1:-install}" in
  install) install_release ;;
  uninstall) uninstall_release ;;
  -h | --help | help) usage ;;
  *) die "unknown command: ${1} (expected install or uninstall)" ;;
esac
