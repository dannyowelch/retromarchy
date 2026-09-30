#!/usr/bin/env bash
# Build dist/ from the release binary. Name and version come from Cargo.toml.
# The binary defaults to target/release/<name>.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

manifest="${root}/Cargo.toml"
name=$(sed -n 's/^name = "\([^"]*\)"/\1/p' "$manifest")
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$manifest")
name="${name%%$'\n'*}"
version="${version%%$'\n'*}"
if [[ -z "$name" || -z "$version" ]]; then
  printf 'Cargo.toml is missing name or version\n' >&2
  exit 1
fi

bin="${1:-${root}/target/release/${name}}"
desktop="${root}/packaging/${name}.desktop"
icon="${root}/packaging/${name}.png"
if [[ ! -f "$bin" ]]; then
  printf 'missing binary: %s\n' "$bin" >&2
  exit 1
fi
if [[ ! -f "$desktop" || ! -f "$icon" ]]; then
  printf 'missing %s desktop file or icon\n' "$name" >&2
  exit 1
fi
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$desktop"
fi

dist="${root}/dist"
rm -rf "$dist"
mkdir -p "$dist"

bundle="${name}-${version}-x86_64-linux"
stage=$(mktemp -d)
mkdir -p "${stage}/${bundle}"
install -m 755 "$bin" "${stage}/${bundle}/${name}"
install -m 644 "$desktop" "${stage}/${bundle}/${name}.desktop"
install -m 644 "$icon" "${stage}/${bundle}/${name}.png"
install -m 644 "${root}/README.md" "${stage}/${bundle}/README.md"
tar -C "$stage" -czf "${dist}/${bundle}.tar.gz" "$bundle"
rm -rf "$stage"

arch_dir="${root}/packaging/arch"
install -m 755 "$bin" "${arch_dir}/${name}"
install -m 644 "$desktop" "${arch_dir}/${name}.desktop"
install -m 644 "$icon" "${arch_dir}/${name}.png"

pkgfile="${name}-${version}-1-x86_64.pkg.tar.zst"
if command -v makepkg >/dev/null 2>&1; then
  chmod a+rX "$root" "${root}/packaging" "$arch_dir"
  chmod a+r "$manifest"
  if [[ "$(id -u)" -eq 0 ]]; then
    if ! id builder >/dev/null 2>&1; then
      useradd --create-home --user-group builder
    fi
    chown -R builder "$arch_dir"
    (cd "$arch_dir" && runuser -u builder -- makepkg -f --noconfirm --nosign)
  else
    (cd "$arch_dir" && makepkg -f --noconfirm --nosign)
  fi
  if [[ ! -f "${arch_dir}/${pkgfile}" ]]; then
    printf 'makepkg did not produce %s\n' "$pkgfile" >&2
    exit 1
  fi
  mv "${arch_dir}/${pkgfile}" "${dist}/${pkgfile}"
else
  dest="${dist}/pkgroot"
  install -Dm755 "$bin" "${dest}/usr/bin/${name}"
  install -Dm644 "$desktop" "${dest}/usr/share/applications/${name}.desktop"
  install -Dm644 "$icon" "${dest}/usr/share/icons/hicolor/256x256/apps/${name}.png"
  printf 'makepkg is not installed; staged %s\n' "$dest" >&2
fi

(
  cd "$dist"
  files=("${bundle}.tar.gz")
  if [[ -f "$pkgfile" ]]; then
    files+=("$pkgfile")
  fi
  sha256sum "${files[@]}" > SHA256SUMS
)
printf 'release assets in %s\n' "$dist"
