#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4
require_tool npm "install Node.js 18 or newer"
require_tool jq "run: brew install jq"

PACKAGE_DIR="$REPO/target/distrib/ownserp-npm-package"
WORKSPACE_VERSION="$(workspace_version "$REPO")"

usage() {
	cat <<'USAGE'
Usage: tools/publish-npm.sh [--dry-run]

Publish the npm package that cargo-dist generated in target/distrib.

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

[[ -f "$PACKAGE_DIR/package.json" ]] || die "missing $PACKAGE_DIR; run tools/release.sh --package first"
PACKAGE_NAME="$(jq -r .name "$PACKAGE_DIR/package.json")"
PACKAGE_VERSION="$(jq -r .version "$PACKAGE_DIR/package.json")"
[[ "$PACKAGE_NAME" == "@devops-infinity/ownserp" ]] || die "the generated package is $PACKAGE_NAME; check the dist config"
[[ "$PACKAGE_VERSION" == "$WORKSPACE_VERSION" ]] || die "the generated package is at $PACKAGE_VERSION but the workspace is at $WORKSPACE_VERSION; rebuild with --package"
if [[ -n "$(git status --porcelain)" ]]; then
	die "the working tree is not clean; commit first"
fi
npm whoami >/dev/null 2>&1 || die "npm is not authenticated; run tools/npm-auth-setup.sh first"

MCP_NAME="io.github.devops-infinity/ownserp"
jq --arg mcpName "$MCP_NAME" 'if .mcpName == $mcpName then . else .mcpName = $mcpName end' \
	"$PACKAGE_DIR/package.json" >"$PACKAGE_DIR/package.json.tmp"
mv "$PACKAGE_DIR/package.json.tmp" "$PACKAGE_DIR/package.json"
jq -e --arg mcpName "$MCP_NAME" '.mcpName == $mcpName' "$PACKAGE_DIR/package.json" >/dev/null ||
	die "could not set mcpName in the generated package"

if [[ "$DRY_RUN" -eq 1 ]]; then
	(cd "$PACKAGE_DIR" && npm pack --dry-run)
	exit 0
fi

LOG="$(mktemp "${TMPDIR:-/tmp}/ownserp-npm-publish.XXXXXX")"
trap 'rm -f -- "$LOG"' EXIT

say INFO "publishing $PACKAGE_NAME $PACKAGE_VERSION"
if ! npm publish "$PACKAGE_DIR" --access public 2>&1 | tee "$LOG"; then
	if grep -q EOTP "$LOG"; then
		die "npm asked for a one-time code; run tools/npm-auth-setup.sh once with a granular token that bypasses two-factor authentication"
	fi
	die "npm publish failed; read the output above"
fi
PUBLISHED=""
for _ in $(seq 1 24); do
	PUBLISHED="$(npm view "$PACKAGE_NAME@$PACKAGE_VERSION" version 2>/dev/null || true)"
	[[ "$PUBLISHED" == "$PACKAGE_VERSION" ]] && break
	sleep 10
done
[[ "$PUBLISHED" == "$PACKAGE_VERSION" ]] || die "npm accepted the publish but the registry still answers ${PUBLISHED:-nothing} for $PACKAGE_VERSION after 240 seconds"
say SUCCESS "published $PACKAGE_NAME $PACKAGE_VERSION"
