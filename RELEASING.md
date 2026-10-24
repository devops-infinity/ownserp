# Releasing OwnSERP

Everything runs on this machine. There is no GitHub Actions workflow, no hosted pipeline, and no release job. `tools/release.sh` does the local half, and `gh` publishes the result only when you run it.

## One-time setup

- Rust 1.99.1 or newer, which `rust-toolchain.toml` pins.
- The gate tools: `cargo install --locked cargo-audit cargo-deny cargo-machete`, plus `shellcheck`, `shfmt`, `gitleaks`, and `jq` from Homebrew.
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

Use `--minor`, `--major`, or `--version X.Y.Z` instead of `--patch` as needed. It sets the version in `Cargo.toml`, the `ownserp-core` requirement, `Cargo.lock`, `server.json`, `mcpb/manifest.json`, the npm package and loader, the Dockerfile label, and the README install snippet, and inserts the `## [X.Y.Z] - DATE` heading in `CHANGELOG.md`.

2. Write the changelog entries under that heading, then commit and merge.

```bash
git add -A && git commit -m "Prepare X.Y.Z"
git push -u origin release/prepare-X-Y-Z
gh pr create --fill --base main
```

3. Package from main. This refuses an empty changelog section, runs `tools/verify.sh`, builds the host binary plus the aarch64, musl, and Windows targets with `cargo-zigbuild`, adds shell completions, the manual page, and the MCPB bundle, writes one archive and checksum per target, and stamps the host archive digest into the npm loader.

```bash
git switch main && git pull
tools/release.sh --package
```

The output is one `dist/ownserp-X.Y.Z-<target>.tar.gz` plus `.sha256` per target, `dist/ownserp-X.Y.Z.mcpb` plus `.sha256`, and `packages/npm/bin/ownserp.js` now pins the host digest. Commit that stamp on a release branch, open a pull request, and merge it.

4. Tag the merged commit, then publish, on your order.

```bash
git switch main && git pull
tools/release.sh --tag
git push origin vX.Y.Z
gh release create vX.Y.Z --title "OwnSERP X.Y.Z" --notes-file <(sed -n '/## \[X.Y.Z\]/,/^## \[/p' CHANGELOG.md | head -n -1)
gh release upload vX.Y.Z dist/ownserp-X.Y.Z-* dist/ownserp-X.Y.Z.mcpb*
tools/publish-npm.sh
```

The tag is created only when the loader pins the archive digest, so the tagged source and the published package agree. Nothing is pushed or published by the release script.

## npm without one-time codes

npm asks for a one-time code on every publish when the account has two-factor authentication and the stored credential is an interactive login. Replace that once with a granular token that bypasses it:

1. Open https://www.npmjs.com/settings/<your-user>/tokens, choose Generate New Token, then Granular Access Token.
2. Allow read and write for the `@devops-infinity/ownserp` package only, set the shortest expiry that covers the release, and tick "Bypass two-factor authentication".
3. Run `tools/npm-auth-setup.sh` and paste the token. It is read with echo off, written to your user npmrc with owner-only permissions, and never printed.

After that, `tools/publish-npm.sh` publishes the loader package and verifies that the registry answers with the same version. It refuses when `packages/npm` and the workspace disagree on the version, and it names the fix when npm still answers with EOTP.

## Install or reinstall locally

```bash
tools/reinstall.sh            # gate, release build, install to ~/.local/bin
tools/reinstall.sh --debug    # debug build, faster to iterate
tools/reinstall.sh --skip-gate --dir /usr/local/bin
```

## The gate by hand

`tools/verify.sh` is the same set the release runs: shell syntax, shellcheck, shfmt, `cargo fmt --check`, check, clippy with warnings denied, tests, doc tests, docs, `cargo audit` with the three reasoned suppressions, `cargo deny check`, `cargo machete`, and a `gitleaks` scan of the tree and of the commit history. Run it before any push.

## What a release does not do

- It does not create or edit a GitHub workflow.
- It does not commit, push, tag a remote, or publish a release. Those are separate commands you run.
- It does not publish to crates.io, npm, or the MCP Registry. Each stays a deliberate command after `tools/registry-auth.sh`, so a publish is never an accident.
