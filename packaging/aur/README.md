# Maintaining the AUR packages

`retromarchy-bin` installs the GitHub release tarball. `retromarchy` builds the tagged source. They are not on the AUR until the first push below.

This repository has no `LICENSE` file. Both packages set `license=('unknown')`, the same placeholder as `packaging/arch/PKGBUILD`. Nothing is installed under `/usr/share/licenses`.

## One-time setup

1. Create an AUR account at <https://aur.archlinux.org/register>.
2. Add your SSH public key to the AUR profile (My Account → SSH Public Key).
3. Create the two package repositories by pushing them once. From a checkout of this repo, with that key used for `aur.archlinux.org`:

   ```bash
   export GIT_SSH_COMMAND="ssh -i ~/.ssh/aur -o IdentitiesOnly=yes"
   for pkg in retromarchy-bin retromarchy; do
     rm -rf "$pkg"
     git -c init.defaultBranch=master clone "ssh://aur@aur.archlinux.org/${pkg}.git"
     cp "packaging/aur/${pkg}/PKGBUILD" "packaging/aur/${pkg}/.SRCINFO" "$pkg/"
     git -C "$pkg" add PKGBUILD .SRCINFO
     git -C "$pkg" commit -m "Initial import of ${pkg}"
     git -C "$pkg" push -u origin master
   done
   ```

   Cloning a package that does not exist yet yields an empty repository. The first push fills it. The AUR only accepts the `master` branch, and the push must include `.SRCINFO`.

4. Generate a dedicated deploy key pair:

   ```bash
   ssh-keygen -t ed25519 -f aur-deploy -N ""
   ```

5. Add the private key (`aur-deploy`) as the GitHub Actions secret `AUR_SSH_PRIVATE_KEY` (Settings → Secrets and variables → Actions). Paste the PEM, including the `BEGIN` and `END` lines. Add `aur-deploy.pub` to the AUR profile as well. Multiple public keys go in that field separated by a newline, so the deploy key can sit next to your own.

After the secret is set, `.github/workflows/aur.yml` pushes both packages itself, including the first push that creates a repository. Step 3 can be left to that run.

## What the workflow does

It runs after the Release workflow completes. GitHub only reads this workflow file from the default branch, so it does nothing until this change is merged. A `release: published` trigger can start before `SHA256SUMS` is uploaded, so this waits until that workflow has finished. The publish job records its tag in the `aur-release-tag` artifact. This workflow reads that tag, downloads `SHA256SUMS` for `retromarchy-<ver>-x86_64-linux.tar.gz`, hashes the tag archive `https://github.com/dannyowelch/retromarchy/archive/refs/tags/v<ver>.tar.gz`, rewrites both PKGBUILDs (`pkgver`, `pkgrel=1`, `sha256sums`), runs `makepkg --printsrcinfo`, and pushes to:

- `ssh://aur@aur.archlinux.org/retromarchy-bin.git`
- `ssh://aur@aur.archlinux.org/retromarchy.git`

If `AUR_SSH_PRIVATE_KEY` is not set, the job prints `AUR_SSH_PRIVATE_KEY is not set; skipping AUR publish.` and exits successfully.

An artifact-only Release run (workflow dispatch with an empty tag) does not publish a GitHub Release. This job skips that case.

Commits are authored as the name and email on this repository's commits unless the `AUR_GIT_NAME` and `AUR_GIT_EMAIL` repository variables are set.

The AUR allows automated PKGBUILD updates at the maintainer's risk. A release that changes dependencies or the license still needs a look before it is pushed, or a follow-up commit.

## Build notes

- Rust edition is 2021. `rust-toolchain.toml` sets `channel = "stable"`. `Cargo.toml` has no `rust-version`. The PKGBUILD exports `RUSTUP_TOOLCHAIN=stable`, which is a no-op on Arch's `cargo` package and selects stable when rustup is installed.
- `gpui-omarchy` 0.1.2 and every other entry in `Cargo.lock` come from crates.io. There are no git dependencies. `prepare()` runs `cargo fetch --locked` (with the host target) so the download happens before `build()`. A git dependency added later is fetched in that same step. `build()` runs `cargo build --release --locked` with `CARGO_TARGET_DIR=target`, matching the release workflow.
- `makedepends` are the release workflow's compile tools that are not already runtime dependencies: `cargo`, `clang`, `git`, and `pkgconf`. `desktop-file-utils` is only used to validate the desktop file while packing GitHub release assets. `LIBCLANG_PATH=/usr/lib` matches the release workflow because gpui's bindgen needs libclang.
- Runtime `depends` and `optdepends` match `packaging/arch/PKGBUILD`.

Refresh the committed files locally from Arch, or from `archlinux:base-devel`:

```bash
packaging/aur/update-pkgbuilds.sh 0.1.1
```
