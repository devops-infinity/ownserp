# Releasing OwnSERP

Everything is prepared on this machine, and the release build runs on a macOS runner. Pushing a `v*` tag starts `.github/workflows/release.yml`, which builds every target and creates the GitHub release. `tools/release.sh` prepares the version and can package locally when you want that.

## One-time setup

- Rust 1.99.1 or newer, which `rust-toolchain.toml` pins.
- The gate tools: `cargo install --locked cargo-audit cargo-deny cargo-machete`, plus `shellcheck`, `shfmt`, `gitleaks`, and `jq` from Homebrew.
- cargo-dist 0.32.0, which builds the archives and installers: `cargo install --locked cargo-dist --version 0.32.0`.
- minisign with the secret key at `~/.minisign/minisign.key`. The matching public key is `minisign.pub` at the repository root, and the release signs `sha256.sum` with it. Without the key the checksums ship unsigned with a warning.
- The cross targets: `rustup target add aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu aarch64-unknown-linux-musl x86_64-unknown-linux-musl x86_64-pc-windows-msvc`, plus `rustup component add llvm-tools-preview`.
- The cross linkers: `cargo install --locked cargo-zigbuild cargo-xwin`, plus `zig` and `zip` from Homebrew.
- `gh auth login` with push access to `devops-infinity/ownserp`, or `GITHUB_TOKEN` in the environment.
- `CARGO_REGISTRY_TOKEN` in the environment for crates.io.
- `MCP_GITHUB_TOKEN`, `GITHUB_PAT`, or `GITHUB_PERSONAL_ACCESS_TOKEN` with the `read:org` scope, so the MCP Registry login needs no browser.
- Optional: a signing key in `git config user.signingkey`, so `--tag` creates a signed tag instead of an annotated one.
- Local hooks, once per checkout: `tools/install-hooks.sh`. The pre-commit hook scans staged content for secrets, and the pre-push hook runs `tools/verify.sh`.

## Refresh the registry sessions

Run this once per shell session before publishing. It reads every credential from the environment and prints only the account name, so no registry asks for a browser login in the middle of a release.

```bash
tools/registry-auth.sh
```

It refreshes the GitHub CLI, npm, the MCP Registry, and the ghcr login, and it reports whether `CARGO_REGISTRY_TOKEN` is set. Nothing is written to the repository or to a command line.

## Cut a release

1. Prepare the version on a branch. The script refuses to run on a dirty tree and never commits.

```bash
git switch main && git pull
git switch -c release/prepare-X-Y-Z
tools/release.sh --prepare --patch
```

Use `--minor`, `--major`, or `--version X.Y.Z` instead of `--patch` as needed. It sets the version in `Cargo.toml`, the `ownserp-core` requirement, `Cargo.lock`, `server.json`, `mcpb/manifest.json`, the Dockerfile label, and the README install snippet, and inserts the `## [X.Y.Z] - DATE` heading in `CHANGELOG.md`.

2. Write the changelog entries under that heading, then commit and merge.

```bash
git add -A && git commit -m "Prepare X.Y.Z"
git push -u origin release/prepare-X-Y-Z
gh pr create --fill --base main
```

3. Tag the merged commit and push the tag. The release workflow builds every target on a macOS runner and creates the GitHub release.

```bash
git switch main && git pull
git tag -a vX.Y.Z -m "OwnSERP X.Y.Z"
git push origin vX.Y.Z
```

The run is at https://github.com/devops-infinity/ownserp/actions/workflows/release.yml. It attaches one archive and checksum per target, the shell and PowerShell installers, `ownserp.rb`, `ownserp-npm-package.tar.gz`, `sha256.sum`, `source.tar.gz`, and the MCPB bundle. The Apple targets compile natively; Linux cross-builds with zig, and Windows with cargo-xwin. When the `MINISIGN_KEY` repository secret is set, the workflow also signs `sha256.sum` and attaches `minisign.pub`.

To build the same files locally instead, run `tools/release.sh --package` on macOS, then `tools/release.sh --tag`. A Linux host skips the Apple targets and refuses a partial set.

4. Publish the other channels after the release exists, on your order.

```bash
gh release download vX.Y.Z --pattern 'ownserp-npm-package.tar.gz' --pattern 'ownserp.rb' --dir dist
mkdir -p target/distrib/ownserp-npm-package
tar -xzf dist/ownserp-npm-package.tar.gz -C target/distrib/ownserp-npm-package --strip-components=1
tools/publish-npm.sh
cargo publish -p ownserp-core --locked
cargo publish -p ownserp --locked
mcp-publisher publish
```

After the release exists, finish the other channels:

- Homebrew: copy `dist/ownserp.rb` into `devops-infinity/homebrew-tap` under `Formula/ownserp.rb`, run `brew audit --strict devops-infinity/tap/ownserp`, then commit and push the tap.
- crates.io: `cargo publish -p ownserp-core --locked`, then `cargo publish -p ownserp --locked`.
- MCP Registry: `mcp-publisher publish`.
- Container: build and push the image, as before.

## npm without one-time codes

npm asks for a one-time code on every publish when the account has two-factor authentication and the stored credential is an interactive login. Replace that once with a granular token that bypasses it:

1. Open https://www.npmjs.com/settings/<your-user>/tokens, choose Generate New Token, then Granular Access Token.
2. Allow read and write for the `@devops-infinity/ownserp` package only, set the shortest expiry that covers the release, and tick "Bypass two-factor authentication".
3. Run `tools/npm-auth-setup.sh` and paste the token. It is read with echo off, written to your user npmrc with owner-only permissions, and never printed.

After that, `tools/publish-npm.sh` publishes the cargo-dist package from `target/distrib` and verifies that the registry answers with the same version. It refuses when the generated package and the workspace disagree on the version, and it names the fix when npm still answers with EOTP.

## Install or reinstall locally

```bash
tools/reinstall.sh            # gate, release build, install to ~/.local/bin
tools/reinstall.sh --debug    # debug build, faster to iterate
tools/reinstall.sh --skip-gate --dir /usr/local/bin
```

## The gate by hand

`tools/verify.sh` is the same set the release runs: shell syntax, shellcheck, shfmt, `cargo fmt --check`, check, clippy with warnings denied, tests, doc tests, docs, `cargo audit` with the three reasoned suppressions, `cargo deny check`, `cargo machete`, and a `gitleaks` scan of the tree and of the commit history. Run it before any push.

## What a release does not do

- It does not run the release workflow. Pushing the tag does that.
- It does not commit, push, tag a remote, or publish a release. Those are separate commands you run.
- It does not push the Homebrew formula. Copy `dist/ownserp.rb` into the tap and push it.
- It does not publish to crates.io, npm, or the MCP Registry. Each stays a deliberate command after `tools/registry-auth.sh`, so a publish is never an accident.
