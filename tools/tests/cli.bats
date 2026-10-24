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
