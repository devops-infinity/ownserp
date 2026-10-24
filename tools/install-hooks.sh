#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4

usage() {
	cat <<'USAGE'
Usage: tools/install-hooks.sh [--remove]

Point git at the repository hooks in .githooks.

  --remove   unset core.hooksPath for this repository
  -h, --help show this
USAGE
}

REMOVE=0
while [[ $# -gt 0 ]]; do
	case "$1" in
	--remove)
		REMOVE=1
		shift
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		usage
		die "unknown option: $1"
		;;
	esac
done

if [[ "$REMOVE" -eq 1 ]]; then
	git config --unset core.hooksPath || true
	log_line SUCCESS "repository hooks removed"
	exit 0
fi

[[ -d "$REPO/.githooks" ]] || die "the .githooks directory is missing"
chmod +x "$REPO"/.githooks/*
git config core.hooksPath .githooks
log_line SUCCESS "repository hooks installed from .githooks"
log_line INFO "pre-commit scans staged content for secrets; pre-push runs tools/verify.sh"
