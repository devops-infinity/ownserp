#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4
shopt -s inherit_errexit

STEP="startup"
LOG="$(mktemp)"

on_signal() {
	log_line FAILED "interrupted during: $STEP"
	rm -f "$LOG"
	trap - INT TERM
	kill -s "$1" "$$"
}
trap 'on_signal INT' INT
trap 'on_signal TERM' TERM
trap 'rm -f "$LOG"' EXIT

run_check() {
	STEP="$1"
	local hint="$2"
	shift 2
	if ! "$@" >"$LOG" 2>&1; then
		log_line FAILED "$STEP"
		tail -n 40 "$LOG" >&2
		die "$hint"
	fi
	log_line SUCCESS "$STEP"
}

SKIPPED=()
skip_check() {
	local name="$1" reason="$2"
	if [[ "${OWN_SERPER_STRICT_GATE:-0}" == "1" ]]; then
		die "$name was skipped ($reason) and OWN_SERPER_STRICT_GATE=1 needs every check to run"
	fi
	SKIPPED+=("$name")
	warn "$reason"
}

SHELL_FILES=(tools/*.sh .githooks/pre-commit .githooks/pre-push)
check_shell_syntax "${SHELL_FILES[@]}"
log_line SUCCESS "shell syntax"

require_tool cargo "install Rust from https://rustup.rs"
require_tool shellcheck "run: brew install shellcheck"
require_tool shfmt "run: brew install shfmt"
for tool in cargo-audit cargo-deny cargo-machete gitleaks; do
	require_tool "$tool" "run: cargo install --locked $tool, or brew install $tool"
done

run_check "shellcheck" "fix the reported shell problem" shellcheck -x "${SHELL_FILES[@]}"
if command -v bats >/dev/null 2>&1; then
	run_check "shell tests" "fix the failing shell test" bats tools/tests/cli.bats
else
	skip_check "shell tests" "bats is not installed, so the shell tests were skipped"
fi
run_check "shfmt" "run: shfmt -w tools" shfmt -d "${SHELL_FILES[@]}"
run_check "formatting" "run: cargo fmt --all" cargo fmt --all -- --check
run_check "check" "cargo check failed" cargo check --workspace --all-targets --all-features --locked --color=never
run_check "lint" "clippy found problems" cargo clippy --workspace --all-targets --all-features --locked --color=never -- -D warnings
if command -v cargo-nextest >/dev/null 2>&1; then
	run_check "tests" "tests failed" cargo nextest run --workspace --all-features --locked --no-tests=warn --color=never
else
	run_check "tests" "tests failed" cargo test --workspace --all-features --locked --color=never
fi
if rustup toolchain list 2>/dev/null | grep -q '^1\.89'; then
	run_check "msrv" "the declared minimum Rust version does not build; fix the code or raise rust-version" cargo +1.89.0 check --workspace --all-targets --all-features --locked --color=never
else
	skip_check "msrv" "Rust 1.89 is not installed, so the MSRV check was skipped"
fi
run_check "doc tests" "doc tests failed" cargo test --workspace --all-features --locked --doc
run_check "docs" "the docs do not build" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
run_check "advisories" "a dependency advisory is open; see deny.toml" cargo audit --deny warnings --ignore RUSTSEC-2026-0370 --ignore RUSTSEC-2026-0371 --ignore RUSTSEC-2026-0372
run_check "policy" "cargo deny failed" cargo deny check
run_check "suppression dates" "a deny.toml suppression is past its recheck date" python3 tools/check-suppressions.py
run_check "unused dependencies" "remove the dependency machete reports" cargo machete
VERSION="$(workspace_version "$REPO")"
manifest_check() {
	local file="$1"
	[[ -f "$file" ]] || die "$file is missing"
	grep -q "\"version\": \"$VERSION\"" "$file" || die "$file does not name version $VERSION"
}
STEP="manifest versions"
manifest_check server.json
manifest_check mcpb/manifest.json
log_line SUCCESS "manifest versions"
if command -v cargo-llvm-cov >/dev/null 2>&1; then
	run_check "coverage" "line coverage is below the 80 percent floor; add a test for the changed code" cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only
else
	skip_check "coverage" "cargo-llvm-cov is not installed, so the coverage floor was skipped"
fi
run_check "secret scan" "a credential pattern is in the tree" gitleaks detect --source . --no-git --redact --exit-code 1 --config .gitleaks.toml
run_check "secret scan (history)" "a credential pattern is in the commit history" gitleaks git --redact --exit-code 1 --config .gitleaks.toml

if ((${#SKIPPED[@]})); then
	log_line WARNING "gate passed with skipped checks: ${SKIPPED[*]}"
else
	log_line SUCCESS "gate passed: $REPO"
fi
