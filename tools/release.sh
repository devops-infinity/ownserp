#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4
shopt -s inherit_errexit

MODE=""
VERSION=""
BUMP=""
DRY_RUN=0
MAKE_TAG=0
DIST_DIR="$REPO/dist"
STAGE=""
TEMP_FILES=()
cleanup() {
	if [[ -n "$STAGE" ]]; then
		rm -rf "$STAGE"
	fi
	if ((${#TEMP_FILES[@]})); then
		rm -f -- "${TEMP_FILES[@]}"
	fi
}
trap cleanup EXIT

usage() {
	cat <<'USAGE'
Usage: tools/release.sh --prepare (--patch | --minor | --major | --version X.Y.Z)
       tools/release.sh --package [--dry-run]
       tools/release.sh --tag [--dry-run]

Everything runs on this machine. Nothing is committed, pushed, or published.

  --prepare        set the next version in Cargo.toml, the ownserp-core
                   requirement, Cargo.lock, server.json, mcpb/manifest.json,
                   the Dockerfile label, and the README install snippet, add
                   the CHANGELOG heading and its compare link, and move any
                   notes under [Unreleased] to the new heading. The diff is
                   left in the working tree for review.
  --package        require a clean tree, run tools/verify.sh, build every
                   cargo-dist target this host supports, write the archives,
                   installers, Homebrew formula, and npm package under dist/,
                   build the MCPB bundle, and install the host binary into
                   ~/.local/bin (override with OWN_SERPER_INSTALL_DIR).
                   Run this on macOS to include the two Darwin targets, and
                   on Windows for the Windows target; a target that needs
                   another host is skipped. A set that skips a target is
                   refused unless OWN_SERPER_ALLOW_PARTIAL=1 is set.
                   Needs cargo-dist, cargo-zigbuild, zig, and zip.
  --tag            require a clean tree and a dist/ packaged for the current
                   version, run the strict gate, then create vX.Y.Z at HEAD
                   (signed when user.signingkey is set); never pushed. The
                   release workflow is the only publisher.
  --patch          bump the patch number (with --prepare)
  --minor          bump the minor number (with --prepare)
  --major          bump the major number (with --prepare)
  --version X.Y.Z  use this exact version instead of a computed one
  --dry-run        print every action and change nothing
  -h, --help       show this

Release order on this machine:
  1. tools/release.sh --prepare --patch, then edit CHANGELOG.md
  2. commit the bump on a release branch and merge it into main
  3. tools/release.sh --package on macOS, so the two Darwin targets are built
  4. tools/release.sh --tag, then push the tag; the release workflow builds,
     signs, and publishes the release
USAGE
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--prepare)
		MODE="prepare"
		shift
		;;
	--package)
		MODE="package"
		shift
		;;
	--patch | --minor | --major)
		BUMP="${1#--}"
		shift
		;;
	--version)
		[[ $# -ge 2 ]] || die "--version needs X.Y.Z"
		VERSION="$2"
		shift 2
		;;
	--tag)
		MAKE_TAG=1
		if [[ -z "$MODE" ]]; then
			MODE="tag"
		fi
		shift
		;;
	--dry-run)
		DRY_RUN=1
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

[[ -n "$MODE" ]] || {
	usage
	die "choose --prepare, --package, or --tag"
}
[[ "$MODE" == "package" && "$MAKE_TAG" -eq 1 ]] && die "--tag runs after --package; run --package, then --tag"
require_tool cargo "install Rust from https://rustup.rs"
require_tool jq "run: brew install jq"

CURRENT="$(workspace_version "$REPO")"
[[ "$CURRENT" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "could not read the workspace version, got: $CURRENT"

bump_version() {
	local current="$1" kind="$2" major minor patch
	IFS=. read -r major minor patch <<<"$current"
	case "$kind" in
	patch) patch=$((patch + 1)) ;;
	minor) minor=$((minor + 1)) && patch=0 ;;
	major) major=$((major + 1)) && minor=0 && patch=0 ;;
	*) die "unknown bump: $kind" ;;
	esac
	printf '%s.%s.%s\n' "$major" "$minor" "$patch"
}

version_gt() {
	local left="$1" right="$2"
	[[ "$(printf '%s\n%s\n' "$left" "$right" | sort -V | tail -n 1)" == "$left" && "$left" != "$right" ]]
}

require_clean_tree() {
	[[ -z "$(git status --porcelain)" ]] || die "the working tree is not clean; commit or stash first"
}

set_manifest_version() {
	local file="$1" version="$2" tmp
	[[ -f "$file" ]] || die "missing $file"
	tmp="$(mktemp "$(dirname -- "$file")/.stamp.XXXXXX")"
	TEMP_FILES+=("$tmp")
	jq --arg version "$version" '.version = $version | (.packages[]?.version) = $version' "$file" >"$tmp"
	mv "$tmp" "$file"
}

changelog_has_entries() {
	local version="$1"
	awk -v needle="## [$version]" '
		index($0, needle) == 1 { inside=1; next }
		inside && /^## / { exit }
		inside && /[^[:space:]]/ { found=1; exit }
		END { exit(found ? 0 : 1) }
	' CHANGELOG.md
}

prepare() {
	local new="$1" date
	require_clean_tree
	version_gt "$new" "$CURRENT" || die "$new is not newer than $CURRENT"
	date="$(date +%F)"
	log_line INFO "preparing $CURRENT -> $new"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		log_line INFO "dry run: would set the version in Cargo.toml, the ownserp-core requirement, Cargo.lock, server.json, mcpb/manifest.json, the Dockerfile label, and the README install snippet"
		log_line INFO "dry run: would add the CHANGELOG heading ## [$new] - $date, move the [Unreleased] notes under it, and add its compare link"
		return 0
	fi
	stamp_version "$REPO" "$CURRENT" "$new"
	cargo metadata --format-version 1 >/dev/null
	set_manifest_version server.json "$new"
	set_manifest_version mcpb/manifest.json "$new"
	if [[ -f Dockerfile ]]; then
		sed -i.bak "s/org.opencontainers.image.version=\"[0-9.]*\"/org.opencontainers.image.version=\"$new\"/" Dockerfile
		rm -f Dockerfile.bak
	fi
	insert_changelog_heading "$new" "$date"
	add_changelog_link "$new" "$CURRENT"
	if ! (verify_version_stamps "$REPO" "$new"); then
		log_line FAILED "the version stamp did not reach every file; undo with: git checkout -- ."
		exit 1
	fi
	log_line SUCCESS "prepared $new"
	log_line INFO "next: write the CHANGELOG entries, commit on a release branch, and merge into main"
}

package() {
	local host target archive stage install_dir bundle tmpstage manifests
	local -a targets=() built=() skipped=()
	require_clean_tree
	changelog_has_entries "$CURRENT" || die "CHANGELOG.md has no entries under ## [$CURRENT]; write them before packaging"
	host="$(host_target)"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		log_line INFO "dry run: would run tools/verify.sh, build every cargo-dist target this host supports, and write the archives, installers, formula, and npm package under $DIST_DIR"
		return 0
	fi
	OWN_SERPER_STRICT_GATE=1 "$REPO/tools/verify.sh"
	require_tool dist "run: cargo install --locked cargo-dist --version 0.32.0"
	require_tool cargo-zigbuild "run: cargo install --locked cargo-zigbuild"
	require_tool zig "run: brew install zig"
	while IFS= read -r target; do
		[[ -n "$target" ]] && targets+=("$target")
	done < <(dist plan --tag="v$CURRENT" --output-format=json | jq -r '.ci.github.artifacts_matrix.include[].targets[]' | sort -u)
	[[ ${#targets[@]} -gt 0 ]] || die "dist plan returned no targets"
	rm -rf -- "$DIST_DIR" target/distrib target/dist-manifests
	mkdir -p "$DIST_DIR" target/distrib target/dist-manifests
	manifests="$REPO/target/dist-manifests"
	for target in "${targets[@]}"; do
		if [[ "$target" == *-apple-darwin && "$host" != *-apple-darwin && -z "${SDKROOT:-}" ]]; then
			skipped+=("$target")
			log_line WARNING "$target needs macOS or an Apple SDK; build it on the Mac"
			continue
		fi
		if [[ "$target" == *-windows-* && "$host" != *-windows-* ]]; then
			skipped+=("$target")
			log_line WARNING "$target needs a Windows host; build it on Windows"
			continue
		fi
		if dist build --tag="v$CURRENT" --artifacts=local --target="$target" --no-local-paths --output-format=json >"$manifests/$target.json"; then
			cp "$manifests/$target.json" "target/distrib/$target-dist-manifest.json"
			built+=("$target")
			log_line SUCCESS "built $target"
		else
			die "$target failed to build"
		fi
	done
	[[ ${#built[@]} -gt 0 ]] || die "no target built"
	dist build --tag="v$CURRENT" --artifacts=global --no-local-paths
	find target/distrib -maxdepth 1 -type f -exec cp -p {} "$DIST_DIR"/ \;
	[[ -f "$DIST_DIR/sha256.sum" ]] || die "missing $DIST_DIR/sha256.sum; dist did not produce the installer checksums"
	if command -v minisign >/dev/null 2>&1 && [[ -f "${HOME}/.minisign/minisign.key" ]]; then
		minisign -Sm "$DIST_DIR/sha256.sum" -s "${HOME}/.minisign/minisign.key" >/dev/null
		cp "$REPO/minisign.pub" "$DIST_DIR/minisign.pub"
		minisign -Vm "$DIST_DIR/sha256.sum" -p "$REPO/minisign.pub" >/dev/null 2>&1 ||
			die "the signature does not verify against $REPO/minisign.pub; check that the signing key matches the published key"
		log_line SUCCESS "signed sha256.sum with minisign"
	else
		log_line WARNING "minisign or ~/.minisign/minisign.key is missing; sha256.sum is unsigned"
	fi
	if ((${#skipped[@]})); then
		log_line WARNING "skipped targets: ${skipped[*]}"
		log_line WARNING "the installers and formula still name every target; build the skipped ones on their own host before publishing"
		if [[ "${OWN_SERPER_ALLOW_PARTIAL:-0}" != "1" ]]; then
			die "built ${#built[@]} of ${#targets[@]} targets; run --package on macOS for the Darwin targets and on Windows for the Windows target, or set OWN_SERPER_ALLOW_PARTIAL=1 to publish this partial set on purpose"
		fi
	fi
	require_tool zip "run: brew install zip"
	archive="$DIST_DIR/ownserp-x86_64-unknown-linux-gnu.tar.gz"
	[[ -f "$archive" ]] || die "missing $archive; the MCPB bundle needs the Linux x86_64 build"
	stage="$REPO/target/package-stage"
	STAGE="$stage"
	rm -rf "$stage"
	mkdir -p "$stage/unpack" "$stage/bundle/server"
	tar -xzf "$archive" -C "$stage/unpack"
	install -m 0755 "$stage/unpack/ownserp-x86_64-unknown-linux-gnu/ownserp" "$stage/bundle/server/ownserp"
	cp mcpb/manifest.json "$stage/bundle/manifest.json"
	find "$stage/bundle" -exec touch -t 198001010000 {} +
	bundle="$DIST_DIR/ownserp-$CURRENT.mcpb"
	(cd "$stage/bundle" && zip -qXr "$bundle" manifest.json server)
	sha256_line_of "$bundle" >"$bundle.sha256"
	log_line SUCCESS "wrote $bundle"
	archive="$DIST_DIR/ownserp-$host.tar.gz"
	if [[ "$host" != *windows* && -f "$archive" ]]; then
		install_dir="${OWN_SERPER_INSTALL_DIR:-$HOME/.local/bin}"
		tmpstage="$stage/install"
		mkdir -p "$tmpstage"
		tar -xzf "$archive" -C "$tmpstage"
		mkdir -p "$install_dir"
		install -m 0755 "$tmpstage/ownserp-$host/ownserp" "$install_dir/.ownserp.new"
		mv -f "$install_dir/.ownserp.new" "$install_dir/ownserp"
		log_line SUCCESS "installed $("$install_dir/ownserp" --version) to $install_dir/ownserp"
	else
		log_line WARNING "no host archive for $host; the local binary was not replaced"
	fi
	[[ -f "$DIST_DIR/ownserp.rb" ]] && log_line INFO "the Homebrew formula is $DIST_DIR/ownserp.rb"
	log_line INFO "next: run tools/release.sh --tag, then push the tag; the release workflow publishes"
}

tag_release() {
	require_clean_tree
	changelog_has_entries "$CURRENT" || die "CHANGELOG.md has no entries under ## [$CURRENT]; write them before tagging"
	require_packaged_dist "$CURRENT" "$DIST_DIR"
	git rev-parse -q --verify "refs/tags/v$CURRENT" >/dev/null && die "tag v$CURRENT already exists"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		log_line INFO "dry run: would run the strict gate and create v$CURRENT at HEAD"
		return 0
	fi
	OWN_SERPER_STRICT_GATE=1 "$REPO/tools/verify.sh"
	if [[ -n "$(git config user.signingkey || true)" ]]; then
		git tag -s "v$CURRENT" -m "OwnSERP $CURRENT"
	else
		git tag -a "v$CURRENT" -m "OwnSERP $CURRENT"
	fi
	log_line SUCCESS "created tag v$CURRENT (not pushed)"
	log_line INFO "push the tag with: git push origin v$CURRENT; .github/workflows/release.yml is the only publisher and builds, signs, and releases from the tag"
}

case "$MODE" in
prepare)
	if [[ -n "$VERSION" ]]; then
		NEXT="$VERSION"
	elif [[ -n "$BUMP" ]]; then
		NEXT="$(bump_version "$CURRENT" "$BUMP")"
	else
		die "--prepare needs --patch, --minor, --major, or --version"
	fi
	[[ "$NEXT" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must look like X.Y.Z, got: $NEXT"
	prepare "$NEXT"
	;;
package)
	[[ -z "$VERSION" && -z "$BUMP" ]] || die "--package packs the version already in Cargo.toml; bump first with --prepare"
	package
	;;
tag)
	[[ -z "$VERSION" && -z "$BUMP" ]] || die "--tag uses the version already in Cargo.toml"
	tag_release
	;;
esac
