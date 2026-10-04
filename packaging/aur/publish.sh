#!/usr/bin/env bash
# Push packaging/aur/retromarchy-bin and packaging/aur/retromarchy to the AUR.
# Exits 0 without publishing when AUR_SSH_PRIVATE_KEY is unset.
# Usage: publish.sh <version>
set -euo pipefail

version="${1:?usage: publish.sh <version>}"
aur=$(cd "$(dirname "$0")" && pwd)

if [[ -z "${AUR_SSH_PRIVATE_KEY:-}" ]]; then
  printf 'AUR_SSH_PRIVATE_KEY is not set; skipping AUR publish.\n'
  exit 0
fi

"$aur/update-pkgbuilds.sh" "$version"
version="${version#v}"

install -d -m 700 "${HOME}/.ssh"
keyfile="${HOME}/.ssh/retromarchy-aur"
known="${HOME}/.ssh/retromarchy-aur.known_hosts"
umask 077
printf '%s\n' "$AUR_SSH_PRIVATE_KEY" | tr -d '\r' > "$keyfile"
chmod 600 "$keyfile"
umask 022
cp "$aur/known_hosts" "$known"
chmod 644 "$known"
export GIT_SSH_COMMAND="ssh -i ${keyfile} -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=${known}"

base="${AUR_PUSH_BASE:-ssh://aur@aur.archlinux.org}"
git_name="${AUR_GIT_NAME:-Danny Welch}"
git_email="${AUR_GIT_EMAIL:-danny.welch@gmail.com}"

publish_one() {
  local pkg="$1"
  local url="${base}/${pkg}.git"
  local work repo log
  work=$(mktemp -d)
  repo="${work}/repo"
  log="${work}/clone.log"
  if git -c init.defaultBranch=master clone "$url" "$repo" >"$log" 2>&1; then
    :
  else
    if grep -qiE 'does not exist|not found|repository not found' "$log"; then
      mkdir -p "$repo"
      git -C "$repo" init -q -b master
      git -C "$repo" remote add origin "$url"
    else
      cat "$log" >&2
      exit 1
    fi
  fi
  git -C "$repo" checkout -q -B master
  cp "$aur/$pkg/PKGBUILD" "$aur/$pkg/.SRCINFO" "$repo/"
  chmod 644 "$repo/PKGBUILD" "$repo/.SRCINFO"
  git -C "$repo" add PKGBUILD .SRCINFO
  if git -C "$repo" diff --cached --quiet; then
    printf '%s is already at %s; nothing to push.\n' "$pkg" "$version"
    return 0
  fi
  git -C "$repo" -c "user.name=${git_name}" -c "user.email=${git_email}" commit -m "Update ${pkg} to ${version}"
  git -C "$repo" push -u origin master
  printf 'pushed %s %s\n' "$pkg" "$version"
}

publish_one retromarchy-bin
publish_one retromarchy
