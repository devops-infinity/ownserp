#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
source "$REPO/tools/lib.sh"
require_bash 4.4
require_tool npm "install Node.js 18 or newer"

NPMRC="${NPM_CONFIG_USERCONFIG:-$HOME/.npmrc}"
KEY="//registry.npmjs.org/:_authToken="

usage() {
	cat <<'USAGE'
Usage: tools/npm-auth-setup.sh

Store a granular npm access token for publishing, so npm publish never asks
for a one-time code.

Create the token once at https://www.npmjs.com/settings/<your-user>/tokens:
choose Granular Access Token, allow read and write for the
@devops-infinity/ownserp package only, set the shortest expiry that covers the
release, and tick "Bypass two-factor authentication".

The token is read from this terminal with echo off. It is written only to
your user npmrc, with owner-only permissions, and never printed.
USAGE
}

case "${1:-}" in
-h | --help)
	usage
	exit 0
	;;
"") ;;
*)
	usage
	die "unknown option: $1"
	;;
esac

say INFO "paste the granular token, then press Enter (input is hidden)"
IFS= read -rs TOKEN
printf '\n' >&2
[[ -n "${TOKEN//[[:space:]]/}" ]] || die "no token entered"

umask 077
SCRATCH="$(mktemp)"
trap 'rm -f "$SCRATCH"' EXIT
if [[ -f "$NPMRC" ]]; then
	grep -vF "$KEY" "$NPMRC" >"$SCRATCH" || true
fi
printf '%s%s\n' "$KEY" "$TOKEN" >>"$SCRATCH"
chmod 600 "$SCRATCH"
set +e
WHO="$(NPM_CONFIG_USERCONFIG="$SCRATCH" npm whoami 2>&1)"
STATUS=$?
set -e
unset TOKEN
if [[ "$STATUS" -ne 0 ]]; then
	die "the token was not accepted, so $NPMRC is unchanged: $WHO"
fi
mv "$SCRATCH" "$NPMRC"
chmod 600 "$NPMRC"
say SUCCESS "npm authenticated as $WHO; publishing no longer needs a one-time code"
