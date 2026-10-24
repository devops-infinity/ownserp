#!/usr/bin/env bats

setup() {
	REPO="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "release help exits zero" {
	run "$REPO/tools/release.sh" --help
	[ "$status" -eq 0 ]
	[[ "$output" == *"Usage: tools/release.sh"* ]]
}

@test "release refuses an unknown option" {
	run "$REPO/tools/release.sh" --nonsense
	[ "$status" -ne 0 ]
	[[ "$output" == *"unknown option"* ]]
}

@test "release prepare needs a bump or a version" {
	run "$REPO/tools/release.sh" --prepare --dry-run
	[ "$status" -ne 0 ]
	[[ "$output" == *"needs --patch"* ]]
}

@test "release package refuses a version flag" {
	run "$REPO/tools/release.sh" --package --version 9.9.9 --dry-run
	[ "$status" -ne 0 ]
	[[ "$output" == *"bump first"* ]]
}

@test "release refuses package and tag in one pass" {
	run "$REPO/tools/release.sh" --package --tag --dry-run
	[ "$status" -ne 0 ]
	[[ "$output" == *"runs after --package"* ]]
}

@test "reinstall help exits zero" {
	run "$REPO/tools/reinstall.sh" --help
	[ "$status" -eq 0 ]
	[[ "$output" == *"Build OwnSERP"* ]]
}

@test "install-hooks help exits zero" {
	run "$REPO/tools/install-hooks.sh" --help
	[ "$status" -eq 0 ]
	[[ "$output" == *"--remove"* ]]
}

@test "npm auth setup help exits zero" {
	run "$REPO/tools/npm-auth-setup.sh" --help
	[ "$status" -eq 0 ]
	[[ "$output" == *"Bypass two-factor authentication"* ]]
}

@test "registry auth help exits zero" {
	run "$REPO/tools/registry-auth.sh" --help
	[ "$status" -eq 0 ]
	[[ "$output" == *"MCP_GITHUB_TOKEN"* ]]
}

@test "the version stamp reaches the manifest, the requirement, and the readme" {
	local tree
	tree="$(mktemp -d)"
	mkdir -p "$tree/crates/ownserp"
	printf '[workspace.package]\nversion = "1.2.3"\n' >"$tree/Cargo.toml"
	printf '[dependencies]\nownserp-core = { path = "../ownserp-core", version = "1.2.0" }\n' >"$tree/crates/ownserp/Cargo.toml"
	printf 'VERSION=1.2.3\n' >"$tree/README.md"
	source "$REPO/tools/lib.sh"
	stamp_version "$tree" 1.2.3 1.3.0
	grep -q '^version = "1.3.0"$' "$tree/Cargo.toml"
	grep -qF 'ownserp-core = { path = "../ownserp-core", version = "1.3.0" }' "$tree/crates/ownserp/Cargo.toml"
	grep -qF 'VERSION=1.3.0' "$tree/README.md"
	rm -rf "$tree"
}

@test "the version check refuses a stale requirement or readme" {
	local tree
	tree="$(mktemp -d)"
	mkdir -p "$tree/crates/ownserp"
	printf '[workspace.package]\nversion = "1.3.0"\n' >"$tree/Cargo.toml"
	printf '[dependencies]\nownserp-core = { path = "../ownserp-core", version = "1.2.0" }\n' >"$tree/crates/ownserp/Cargo.toml"
	printf 'VERSION=1.2.3\n' >"$tree/README.md"
	source "$REPO/tools/lib.sh"
	run verify_version_stamps "$tree" 1.3.0
	[ "$status" -ne 0 ]
	[[ "$output" == *"crates/ownserp/Cargo.toml"* ]]
	[[ "$output" == *"README.md"* ]]
	rm -rf "$tree"
}
