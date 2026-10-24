#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4

usage() {
	cat <<'USAGE'
Usage: tools/registry-auth.sh

Refresh the registry sessions used by the release steps, taking every
credential from the environment. Nothing is printed except the account name.

  GitHub CLI        GITHUB_TOKEN, read by gh
  MCP Registry      MCP_GITHUB_TOKEN, else GITHUB_PAT, else
                    GITHUB_PERSONAL_ACCESS_TOKEN
  npm               the token already stored in the user npmrc
  crates.io         CARGO_REGISTRY_TOKEN
  ghcr              GITHUB_TOKEN, through gh auth token

Run it once per shell session before publishing. It never writes a secret to
the repository, a command line, or the terminal.
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

if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
	log_line SUCCESS "GitHub CLI is authenticated as $(gh api user --jq .login 2>/dev/null || echo an account)"
else
	warn "GitHub CLI is not authenticated; set GITHUB_TOKEN or run gh auth login"
fi

if command -v npm >/dev/null 2>&1; then
	if who="$(npm whoami 2>/dev/null)"; then
		log_line SUCCESS "npm is authenticated as $who"
	else
		warn "npm is not authenticated; run tools/npm-auth-setup.sh once"
	fi
fi

if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
	log_line SUCCESS "CARGO_REGISTRY_TOKEN is set for crates.io"
else
	warn "CARGO_REGISTRY_TOKEN is not set; cargo publish will refuse"
fi

if command -v mcp-publisher >/dev/null 2>&1; then
	source_var=""
	for candidate in MCP_GITHUB_TOKEN GITHUB_PAT GITHUB_PERSONAL_ACCESS_TOKEN; do
		if [[ -n "${!candidate:-}" ]]; then
			source_var="$candidate"
			break
		fi
	done
	if [[ -n "$source_var" ]]; then
		if MCP_GITHUB_TOKEN="${!source_var}" mcp-publisher login github >/dev/null 2>&1; then
			log_line SUCCESS "MCP Registry session refreshed from $source_var"
		else
			warn "MCP Registry login failed with $source_var; the token needs the read:org scope"
		fi
	else
		warn "no GitHub PAT in the environment for the MCP Registry"
	fi
fi

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
	login="$(gh api user --jq .login 2>/dev/null || true)"
	if [[ -n "$login" ]] && gh auth token 2>/dev/null | docker login ghcr.io -u "$login" --password-stdin >/dev/null 2>&1; then
		log_line SUCCESS "ghcr login refreshed from GITHUB_TOKEN"
	else
		warn "ghcr login was not refreshed; pushes will need docker login"
	fi
fi

log_line INFO "credentials stay in the environment; no secret is written by this script"
