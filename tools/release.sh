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
                   the npm package and loader, the Dockerfile label, and the
                   README install snippet, and add the CHANGELOG heading.
                   The diff is left in the working tree for review.
  --package        require a clean tree, run tools/verify.sh, build the host
                   binary plus the aarch64, musl, and Windows targets, write
                   one archive and sha256 file per target, add shell
                   completions, the manual page, and the MCPB bundle, install
                   the host binary into ~/.local/bin (override with
                   OWN_SERPER_INSTALL_DIR), and stamp the host archive digest
                   into the npm loader. Commit that stamp before tagging.
                   Needs cargo-zigbuild, zig, and zip.
  --tag            require a clean tree and an archive whose digest the npm
                   loader already pins, then create vX.Y.Z at HEAD (signed
                   when user.signingkey is set); never pushed
  --patch          bump the patch number (with --prepare)
  --minor          bump the minor number (with --prepare)
  --major          bump the major number (with --prepare)
  --version X.Y.Z  use this exact version instead of a computed one
  --dry-run        print every action and change nothing
  -h, --help       show this

Release order on this machine:
  1. tools/release.sh --prepare --patch, then edit CHANGELOG.md
  2. commit the bump on a release branch and merge it into main
  3. tools/release.sh --package, then commit the npm loader stamp and merge
  4. tools/release.sh --tag, then push the tag and publish
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
[[ "$MODE" == "package" && "$MAKE_TAG" -eq 1 ]] && die "--tag runs after --package; run --package, commit the loader stamp, then --tag"
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

insert_changelog_heading() {
	local version="$1" date="$2" tmp
	tmp="$(mktemp "$(dirname -- CHANGELOG.md)/.stamp.XXXXXX")"
	TEMP_FILES+=("$tmp")
	awk -v heading="## [$version] - $date" '
		!done && /^## \[/ { print heading; print ""; done=1 }
		{ print }
		END { if (!done) { print ""; print heading; print "" } }
	' CHANGELOG.md >"$tmp"
	mv "$tmp" CHANGELOG.md
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
	say INFO "preparing $CURRENT -> $new"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		say INFO "dry run: would set the version in Cargo.toml, the ownserp-core requirement, Cargo.lock, server.json, mcpb/manifest.json, the npm package and loader, the Dockerfile label, and the README install snippet"
		say INFO "dry run: would add the CHANGELOG heading ## [$new] - $date"
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
	if [[ -f packages/npm/package.json ]]; then
		set_manifest_version packages/npm/package.json "$new"
		sed -i.bak "s/^const VERSION = \".*\";/const VERSION = \"$new\";/" packages/npm/bin/ownserp.js
		rm -f packages/npm/bin/ownserp.js.bak
	fi
	insert_changelog_heading "$new" "$date"
	verify_version_stamps "$REPO" "$new"
	say SUCCESS "prepared $new"
	say INFO "next: write the CHANGELOG entries, commit on a release branch, and merge into main"
}

package() {
	local host stage extra archive digest loader target binary
	require_clean_tree
	changelog_has_entries "$CURRENT" || die "CHANGELOG.md has no entries under ## [$CURRENT]; write them before packaging"
	host="$(host_target)"
	archive="$DIST_DIR/ownserp-$CURRENT-$host.tar.gz"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		say INFO "dry run: would run tools/verify.sh, build $host plus the cross targets, and write one archive per target under $DIST_DIR"
		return 0
	fi
	"$REPO/tools/verify.sh"
	require_tool cargo-zigbuild "run: cargo install cargo-zigbuild"
	require_tool zig "run: brew install zig"
	mkdir -p "$DIST_DIR"
	STAGE="$(mktemp -d)"
	extra="$STAGE/extra"
	mkdir "$extra"
	cargo build --profile dist --locked --bin ownserp
	install -m 0755 target/dist/ownserp "$extra/ownserp"
	"$extra/ownserp" completions bash >"$extra/ownserp.bash"
	"$extra/ownserp" completions zsh >"$extra/_ownserp"
	"$extra/ownserp" completions fish >"$extra/ownserp.fish"
	"$extra/ownserp" man >"$extra/ownserp.1"
	rm -f "$extra/ownserp"
	local -a targets=("$host")
	for target in aarch64-unknown-linux-gnu x86_64-unknown-linux-musl x86_64-pc-windows-gnu; do
		[[ "$target" == "$host" ]] || targets+=("$target")
	done
	for target in "${targets[@]}"; do
		if [[ "$target" == "$host" ]]; then
			binary="target/dist/ownserp"
			if [[ "$host" == *windows* ]]; then
				binary="$binary.exe"
			fi
		elif [[ "$target" == "aarch64-unknown-linux-gnu" ]]; then
			cargo zigbuild --target "$target.2.28" --profile dist --locked --bin ownserp
			binary="target/$target/dist/ownserp"
		else
			cargo zigbuild --target "$target" --profile dist --locked --bin ownserp
			if [[ "$target" == "x86_64-pc-windows-gnu" ]]; then
				binary="target/$target/dist/ownserp.exe"
			else
				binary="target/$target/dist/ownserp"
			fi
		fi
		[[ -x "$binary" ]] || die "the build did not produce $binary"
		stage="$STAGE/$target"
		mkdir -p "$stage"
		if [[ "$target" == "x86_64-pc-windows-gnu" ]]; then
			install -m 0755 "$binary" "$stage/ownserp.exe"
		else
			install -m 0755 "$binary" "$stage/ownserp"
		fi
		cp "$extra"/* "$stage/"
		cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE server.json "$stage/"
		archive="$DIST_DIR/ownserp-$CURRENT-$target.tar.gz"
		tar -czf "$archive" -C "$stage" .
		sha256_line_of "$archive" >"$archive.sha256"
		say SUCCESS "wrote $archive"
		say SUCCESS "wrote $archive.sha256"
	done
	archive="$DIST_DIR/ownserp-$CURRENT-$host.tar.gz"
	stage="$STAGE/smoke"
	mkdir -p "$stage"
	tar -xzf "$archive" -C "$stage"
	"$stage/ownserp" --version >/dev/null
	say SUCCESS "the host archive unpacks and its binary runs"
	local install_dir="${OWN_SERPER_INSTALL_DIR:-$HOME/.local/bin}"
	if [[ "$host" != *windows* ]]; then
		mkdir -p "$install_dir"
		install -m 0755 target/dist/ownserp "$install_dir/.ownserp.new"
		mv -f "$install_dir/.ownserp.new" "$install_dir/ownserp"
		say SUCCESS "installed $("$install_dir/ownserp" --version) to $install_dir/ownserp"
	fi
	[[ "$host" == *linux* ]] || die "the MCPB bundle needs a Linux host to build"
	require_tool zip "run: brew install zip"
	local bundle="$DIST_DIR/ownserp-$CURRENT.mcpb"
	local bundle_stage="$STAGE/bundle"
	mkdir -p "$bundle_stage/server"
	cp mcpb/manifest.json "$bundle_stage/manifest.json"
	install -m 0755 target/dist/ownserp "$bundle_stage/server/ownserp"
	(cd "$bundle_stage" && zip -qr "$bundle" manifest.json server)
	sha256_line_of "$bundle" >"$bundle.sha256"
	say SUCCESS "wrote $bundle"
	say SUCCESS "wrote $bundle.sha256"
	loader="$REPO/packages/npm/bin/ownserp.js"
	digest="$(sha256_of "$archive")"
	if [[ -f "$loader" ]] && ! grep -q "const DIGEST = \"$digest\";" "$loader"; then
		sed "s/^const DIGEST = \".*\";/const DIGEST = \"$digest\";/" "$loader" >"$loader.tmp"
		mv "$loader.tmp" "$loader"
		say INFO "the npm loader now pins $digest; commit it and merge before --tag"
	else
		say SUCCESS "the npm loader already pins $digest"
	fi
	say INFO "next: commit the loader stamp, merge it, then run tools/release.sh --tag"
}

tag_release() {
	require_clean_tree
	changelog_has_entries "$CURRENT" || die "CHANGELOG.md has no entries under ## [$CURRENT]; write them before tagging"
	local target archive digest loader="$REPO/packages/npm/bin/ownserp.js"
	target="$(host_target)"
	archive="$DIST_DIR/ownserp-$CURRENT-$target.tar.gz"
	[[ -f "$archive" ]] || die "missing $archive; run tools/release.sh --package first"
	digest="$(sha256_of "$archive")"
	if [[ -f "$loader" ]] && ! grep -q "const DIGEST = \"$digest\";" "$loader"; then
		die "the npm loader does not pin $digest; commit the loader stamp from --package before tagging"
	fi
	git rev-parse -q --verify "refs/tags/v$CURRENT" >/dev/null && die "tag v$CURRENT already exists"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		say INFO "dry run: would create v$CURRENT at HEAD with $archive"
		return 0
	fi
	if [[ -n "$(git config user.signingkey || true)" ]]; then
		git tag -s "v$CURRENT" -m "OwnSERP $CURRENT"
	else
		git tag -a "v$CURRENT" -m "OwnSERP $CURRENT"
	fi
	say SUCCESS "created tag v$CURRENT (not pushed)"
	say INFO "publish with: gh release create v$CURRENT \"$archive\" \"$archive.sha256\" --title \"OwnSERP $CURRENT\""
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
