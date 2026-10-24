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

host_target() {
	rustc -vV | awk '/^host:/{print $2}'
}
