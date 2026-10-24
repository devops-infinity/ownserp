# Releasing OwnSERP

Everything runs on this machine. There is no GitHub Actions workflow, no hosted pipeline, and no release job. `tools/release.sh` does the local half, and `gh` publishes the result only when you run it.

## One-time setup

- Rust 1.99.1 or newer, which `rust-toolchain.toml` pins.
- The gate tools: `cargo install --locked cargo-audit cargo-deny cargo-machete`, plus `shellcheck`, `shfmt`, `gitleaks`, and `jq` from Homebrew.
- `gh auth login` with push access to `devops-infinity/ownserp`.
- Optional: a signing key in `git config user.signingkey`, so `--tag` creates a signed tag instead of an annotated one.
- Local hooks, once per checkout: `tools/install-hooks.sh`. The pre-commit hook scans staged content for secrets, and the pre-push hook runs `tools/verify.sh`.

## Cut a release

1. Prepare the version on a branch. The script refuses to run on a dirty tree and never commits.

```bash
git switch main && git pull
git switch -c sazzad/prepare-X-Y-Z
tools/release.sh --prepare --patch
```

Use `--minor`, `--major`, or `--version X.Y.Z` instead of `--patch` as needed. It sets the version in `Cargo.toml`, refreshes `Cargo.lock`, sets it in `server.json` and `mcpb/manifest.json`, and inserts the `## [X.Y.Z] - DATE` heading in `CHANGELOG.md`.

2. Write the changelog entries under that heading, then commit and merge.

```bash
git add -A && git commit -m "Prepare X.Y.Z"
git push -u origin sazzad/prepare-X-Y-Z
gh pr create --fill --base main
```

3. Package from main. This refuses an empty changelog section, runs `tools/verify.sh`, builds the release binary, and writes the archive and its checksum.

```bash
git switch main && git pull
tools/release.sh --package --tag
```

The output is `dist/ownserp-X.Y.Z-<target>.tar.gz` plus `.sha256`, and the tag `vX.Y.Z`. Nothing is pushed.

4. Publish, on your order.

```bash
gh release create vX.Y.Z dist/ownserp-X.Y.Z-<target>.tar.gz dist/ownserp-X.Y.Z-<target>.tar.gz.sha256 \
  --title "OwnSERP X.Y.Z" --notes-file <(sed -n '/## \[X.Y.Z\]/,/^## \[/p' CHANGELOG.md | head -n -1)
git push origin main
git push origin vX.Y.Z
```

## Install or reinstall locally

```bash
tools/reinstall.sh            # gate, release build, install to ~/.local/bin
tools/reinstall.sh --debug    # debug build, faster to iterate
tools/reinstall.sh --skip-gate --dir /usr/local/bin
```

## The gate by hand

`tools/verify.sh` is the same set the release runs: shell syntax, shellcheck, shfmt, `cargo fmt --check`, check, clippy with warnings denied, tests, doc tests, docs, `cargo audit` with the three reasoned suppressions, `cargo deny check`, `cargo machete`, and a `gitleaks` scan of the tree. Run it before any push.

## What a release does not do

- It does not create or edit a GitHub workflow.
- It does not commit, push, tag a remote, or publish a release. Those are separate commands you run.
- It does not publish to crates.io, npm, or the MCP Registry. Each needs your account and a first-publish token, so each stays a deliberate step.
