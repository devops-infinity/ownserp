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

@test "the changelog heading keeps unreleased on top and moves the notes under the new version" {
	local tree
	tree="$(mktemp -d)"
	printf '# Changelog\n\n## [Unreleased]\n\n### Fixed\n\n- a fresh fix\n\n## [1.3.1] - 2026-10-25\n\n### Fixed\n\n- an older fix\n' >"$tree/CHANGELOG.md"
	pushd "$tree" >/dev/null
	source "$REPO/tools/lib.sh"
	insert_changelog_heading 1.3.2 2026-10-26
	popd >/dev/null
	run grep -n '^## ' "$tree/CHANGELOG.md"
	[ "$status" -eq 0 ]
	[[ "${lines[0]}" == *"## [Unreleased]"* ]]
	[[ "${lines[1]}" == *"## [1.3.2] - 2026-10-26"* ]]
	[[ "${lines[2]}" == *"## [1.3.1] - 2026-10-25"* ]]
	grep -A5 '^## \[1.3.2\]' "$tree/CHANGELOG.md" | grep -q 'a fresh fix'
	rm -rf "$tree"
}

@test "the changelog compare link is added once" {
	local tree
	tree="$(mktemp -d)"
	printf '# Changelog\n' >"$tree/CHANGELOG.md"
	pushd "$tree" >/dev/null
	source "$REPO/tools/lib.sh"
	add_changelog_link 1.3.2 1.3.1
	add_changelog_link 1.3.2 1.3.1
	popd >/dev/null
	[ "$(grep -c '^\[1.3.2\]: ' "$tree/CHANGELOG.md")" -eq 1 ]
	grep -q 'compare/v1.3.1...v1.3.2' "$tree/CHANGELOG.md"
	rm -rf "$tree"
}

@test "the shell syntax check fails and names the broken file" {
	local tree
	tree="$(mktemp -d)"
	printf '#!/usr/bin/env bash\nset -e\n' >"$tree/good.sh"
	printf '#!/usr/bin/env bash\nif then\n' >"$tree/broken.sh"
	source "$REPO/tools/lib.sh"
	run check_shell_syntax "$tree/good.sh"
	[ "$status" -eq 0 ]
	run check_shell_syntax "$tree/good.sh" "$tree/broken.sh"
	[ "$status" -ne 0 ]
	[[ "$output" == *"syntax error in $tree/broken.sh"* ]]
	rm -rf "$tree"
}

@test "the tag step refuses a dist built for another version" {
	local tree
	tree="$(mktemp -d)"
	: >"$tree/sha256.sum"
	source "$REPO/tools/lib.sh"
	run require_packaged_dist 1.3.2 "$tree"
	[ "$status" -ne 0 ]
	[[ "$output" == *"ownserp-1.3.2.mcpb"* ]]
	touch "$tree/ownserp-1.3.2.mcpb"
	run require_packaged_dist 1.3.2 "$tree"
	[ "$status" -eq 0 ]
	rm -rf "$tree"
}

@test "registry auth passes the MCP token through the environment" {
	local tree
	tree="$(mktemp -d)"
	cat >"$tree/mcp-publisher" <<'EOF'
#!/usr/bin/env bash
printf '%s' "$MCP_GITHUB_TOKEN" >"$TOKEN_OUT"
EOF
	for tool in gh npm docker; do
		printf '#!/usr/bin/env bash\nexit 1\n' >"$tree/$tool"
	done
	chmod +x "$tree/mcp-publisher" "$tree/gh" "$tree/npm" "$tree/docker"
	export MCP_GITHUB_TOKEN=token-value
	export TOKEN_OUT="$tree/token"
	export PATH="$tree:$PATH"
	run "$REPO/tools/registry-auth.sh"
	[ "$status" -eq 0 ]
	[ "$(cat "$tree/token")" = "token-value" ]
	run grep -q 'env MCP_GITHUB_TOKEN=' "$REPO/tools/registry-auth.sh"
	[ "$status" -ne 0 ]
	rm -rf "$tree"
}

@test "the git hooks do not require bash 4.4 and the push checks the tree" {
	run grep -q 'require_bash' "$REPO/.githooks/pre-commit" "$REPO/.githooks/pre-push"
	[ "$status" -ne 0 ]
	grep -q 'status --porcelain' "$REPO/.githooks/pre-push"
}

@test "the release workflow validates the tag and publishes from a draft" {
	grep -q 'cancel-in-progress: false' "$REPO/.github/workflows/release.yml"
	grep -q 'persist-credentials: false' "$REPO/.github/workflows/release.yml"
	grep -qF "gh release create \"\$TAG\" --draft" "$REPO/.github/workflows/release.yml"
	grep -qF "gh release edit \"\$TAG\" --draft=false" "$REPO/.github/workflows/release.yml"
	grep -qF "[[ \"\$TAG\" =~ ^v[0-9]+\.[0-9]+\.[0-9]+\$ ]]" "$REPO/.github/workflows/release.yml"
	grep -q '::warning::MINISIGN_KEY is not set' "$REPO/.github/workflows/release.yml"
	grep -q '>> "dist/sha256.sum"' "$REPO/.github/workflows/release.yml"
}

@test "the dist config does not promise a homebrew publish job" {
	run grep -q '^publish-jobs' "$REPO/Cargo.toml"
	[ "$status" -ne 0 ]
	run grep -q '^tap =' "$REPO/Cargo.toml"
	[ "$status" -ne 0 ]
}

@test "npm publishing needs a supported Node release" {
	grep -q 'Node.js 22 or newer' "$REPO/tools/publish-npm.sh"
	grep -q 'not byte-identical' "$REPO/tools/publish-npm.sh"
}
