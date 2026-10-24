#!/usr/bin/env bash

say() { printf '%s [%s] %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "$2" >&2; }
warn() { say WARNING "$1"; }
die() {
	say FAILED "$1"
	exit 1
}

version_at_least() {
	local have="$1" want="$2" index left right
	local -a have_parts=() want_parts=()
	[[ "$have" =~ ^[0-9]+(\.[0-9]+)*$ && "$want" =~ ^[0-9]+(\.[0-9]+)*$ ]] || return 1
	IFS=. read -r -a have_parts <<<"$have"
	IFS=. read -r -a want_parts <<<"$want"
	for index in 0 1 2; do
		left="${have_parts[index]:-0}"
		right="${want_parts[index]:-0}"
		if ((10#$left > 10#$right)); then
			return 0
		fi
		if ((10#$left < 10#$right)); then
			return 1
		fi
	done
	return 0
}

require_bash() {
	if version_at_least "${BASH_VERSINFO[0]}.${BASH_VERSINFO[1]}" "$1"; then
		return 0
	fi
	die "bash $1 or newer is required, but this shell is $BASH_VERSION"
}

require_tool() {
	local tool="$1" hint="$2"
	command -v "$tool" >/dev/null 2>&1 && return 0
	die "$tool is not installed; $hint"
}

workspace_root() {
	cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd
}

workspace_version() {
	local manifest="$1/Cargo.toml"
	awk '/^\[workspace\.package\]/{found=1; next} found && /^version = /{gsub(/[",]/, "", $3); print $3; exit}' "$manifest"
}

sha256_of() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | awk '{print $1}'
	else
		shasum -a 256 "$1" | awk '{print $1}'
	fi
}

verify_version_stamps() {
	local root="$1" want="$2" ok=0 file actual
	actual="$(workspace_version "$root")"
	if [[ "$actual" != "$want" ]]; then
		warn "Cargo.toml reports $actual"
		ok=1
	fi
	for file in server.json mcpb/manifest.json packages/npm/package.json; do
		[[ -f "$root/$file" ]] || continue
		actual="$(jq -r .version "$root/$file")"
		if [[ "$actual" != "$want" ]]; then
			warn "$file reports $actual"
			ok=1
		fi
	done
	if [[ -f "$root/packages/npm/bin/ownserp.js" ]] && ! grep -q "const VERSION = \"$want\";" "$root/packages/npm/bin/ownserp.js"; then
		warn "packages/npm/bin/ownserp.js does not report $want"
		ok=1
	fi
	if [[ -f "$root/Dockerfile" ]] && ! grep -q "org.opencontainers.image.version=\"$want\"" "$root/Dockerfile"; then
		warn "Dockerfile does not label $want"
		ok=1
	fi
	[[ "$ok" -eq 0 ]] || die "the version stamp did not reach every file for $want"
}

host_target() {
	rustc -vV | awk '/^host:/{print $2}'
}
