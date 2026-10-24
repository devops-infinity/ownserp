#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4
require_tool npm "install Node.js 18 or newer"
require_tool jq "run: brew install jq"

PACKAGE_DIR="$REPO/packages/npm"
WORKSPACE_VERSION="$(workspace_version "$REPO")"
PACKAGE_VERSION="$(jq -r .version "$PACKAGE_DIR/package.json")"
PACKAGE_NAME="$(jq -r .name "$PACKAGE_DIR/package.json")"

usage() {
	cat <<'USAGE'
Usage: tools/publish-npm.sh [--dry-run]

Publish the npm loader package at packages/npm.

  --dry-run  pack and print the file list, publish nothing
  -h, --help show this
USAGE
}

DRY_RUN=0
case "${1:-}" in
--dry-run)
	DRY_RUN=1
	;;
-h | --help)
	usage
	exit 0
	;;
"")
	;;
*)
	usage
	die "unknown option: $1"
	;;
esac

[[ "$PACKAGE_VERSION" == "$WORKSPACE_VERSION" ]] || die "packages/npm is at $PACKAGE_VERSION but the workspace is at $WORKSPACE_VERSION; run tools/release.sh --prepare first"
[[ "$(jq -r .mcpName "$PACKAGE_DIR/package.json")" == "io.github.devops-infinity/ownserp" ]] || die "package.json lost its mcpName"
if [[ -n "$(git status --porcelain)" ]]; then
	die "the working tree is not clean; commit first"
fi
npm whoami >/dev/null 2>&1 || die "npm is not authenticated; run tools/npm-auth-setup.sh first"

cd "$PACKAGE_DIR"
node --check bin/ownserp.js || die "the loader has a syntax error"

if [[ "$DRY_RUN" -eq 1 ]]; then
	npm pack --dry-run
	exit 0
fi

say INFO "publishing $PACKAGE_NAME $PACKAGE_VERSION"
if ! npm publish --access public 2>&1 | tee /tmp/ownserp-npm-publish.log; then
	if grep -q EOTP /tmp/ownserp-npm-publish.log; then
		die "npm asked for a one-time code; run tools/npm-auth-setup.sh once with a granular token that bypasses two-factor authentication"
	fi
	die "npm publish failed; read the output above"
fi
PUBLISHED="$(npm view "$PACKAGE_NAME" version 2>/dev/null || true)"
[[ "$PUBLISHED" == "$PACKAGE_VERSION" ]] || die "the publish reported success but the registry answers $PUBLISHED"
say SUCCESS "published $PACKAGE_NAME $PACKAGE_VERSION"
