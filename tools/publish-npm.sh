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
verify_version_stamps "$REPO" "$PACKAGE_VERSION"
if [[ -n "$(git status --porcelain)" ]]; then
	die "the working tree is not clean; commit first"
fi
npm whoami >/dev/null 2>&1 || die "npm is not authenticated; run tools/npm-auth-setup.sh first"

ARCHIVE="$REPO/dist/ownserp-$PACKAGE_VERSION-x86_64-unknown-linux-gnu.tar.gz"
[[ -f "$ARCHIVE" ]] || die "missing $ARCHIVE; run tools/release.sh --package first"
DIGEST="$(sha256_of "$ARCHIVE")"
[[ "$DIGEST" =~ ^[0-9a-f]{64}$ ]] || die "could not read the archive digest"

STAMP_TMP="$(mktemp "$PACKAGE_DIR/bin/.digest.XXXXXX")"
LOG="$(mktemp "${TMPDIR:-/tmp}/ownserp-npm-publish.XXXXXX")"
trap 'rm -f -- "$STAMP_TMP" "$LOG"' EXIT
sed "s/^const DIGEST = \".*\";/const DIGEST = \"$DIGEST\";/" \
	"$PACKAGE_DIR/bin/ownserp.js" >"$STAMP_TMP"
mv "$STAMP_TMP" "$PACKAGE_DIR/bin/ownserp.js"

cd "$PACKAGE_DIR"
node --check bin/ownserp.js || die "the loader has a syntax error"

if [[ "$DRY_RUN" -eq 1 ]]; then
	npm pack --dry-run
	exit 0
fi

say INFO "publishing $PACKAGE_NAME $PACKAGE_VERSION"
if ! npm publish --access public 2>&1 | tee "$LOG"; then
	if grep -q EOTP "$LOG"; then
		die "npm asked for a one-time code; run tools/npm-auth-setup.sh once with a granular token that bypasses two-factor authentication"
	fi
	die "npm publish failed; read the output above"
fi
PUBLISHED=""
for _ in 1 2 3 4 5 6 7 8 9 10; do
	PUBLISHED="$(npm view "$PACKAGE_NAME@$PACKAGE_VERSION" version 2>/dev/null || true)"
	[[ "$PUBLISHED" == "$PACKAGE_VERSION" ]] && break
	sleep 10
done
[[ "$PUBLISHED" == "$PACKAGE_VERSION" ]] || die "npm accepted the publish but the registry still answers ${PUBLISHED:-nothing} for $PACKAGE_VERSION after 100 seconds"
say SUCCESS "published $PACKAGE_NAME $PACKAGE_VERSION"
