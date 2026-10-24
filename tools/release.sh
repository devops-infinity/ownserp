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
trap 'if [[ -n "$STAGE" ]]; then rm -rf "$STAGE"; fi' EXIT

usage() {
	cat <<'USAGE'
Usage: tools/release.sh --prepare (--patch | --minor | --major | --version X.Y.Z)
       tools/release.sh --package [--tag] [--dry-run]

Everything runs on this machine. Nothing is committed, pushed, or published.

  --prepare        set the next version in Cargo.toml, Cargo.lock, server.json,
                   and mcpb/manifest.json, and add the CHANGELOG heading.
                   The diff is left in the working tree for review.
  --package        require a clean tree, run tools/verify.sh, build the release
                   binary, and write dist/ownserp-X.Y.Z-<target>.tar.gz with a
                   sha256 file.
  --patch          bump the patch number (with --prepare)
  --minor          bump the minor number (with --prepare)
  --major          bump the major number (with --prepare)
  --version X.Y.Z  use this exact version instead of a computed one
  --tag            create the annotated tag vX.Y.Z at the packaged commit
                   (signed when user.signingkey is set); never pushed
  --dry-run        print every action and change nothing
  -h, --help       show this

Release order on this machine:
  1. tools/release.sh --prepare --patch, then edit CHANGELOG.md
  2. commit the bump on a release branch and merge it into main
  3. tools/release.sh --package --tag
  4. publish the archive and tag with gh, on your order
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
	die "choose --prepare or --package"
}
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
	local file="$1" version="$2"
	[[ -f "$file" ]] || die "missing $file"
	jq --arg version "$version" '.version = $version | (.packages[]?.version) = $version' "$file" >"$file.tmp"
	mv "$file.tmp" "$file"
}

insert_changelog_heading() {
	local version="$1" date="$2"
	awk -v heading="## [$version] - $date" '
		!done && /^## \[/ { print heading; print ""; done=1 }
		{ print }
		END { if (!done) { print ""; print heading; print "" } }
	' CHANGELOG.md >CHANGELOG.md.tmp
	mv CHANGELOG.md.tmp CHANGELOG.md
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
		say INFO "dry run: would set the version in Cargo.toml, Cargo.lock, server.json, mcpb/manifest.json, and the npm package"
		say INFO "dry run: would add the CHANGELOG heading ## [$new] - $date"
		return 0
	fi
	sed -i.bak "s/^version = \"$CURRENT\"$/version = \"$new\"/" Cargo.toml
	rm -f Cargo.toml.bak
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
	say SUCCESS "prepared $new"
	say INFO "next: write the CHANGELOG entries, commit on a release branch, and merge into main"
}

package() {
	local target archive stage
	require_clean_tree
	changelog_has_entries "$CURRENT" || die "CHANGELOG.md has no entries under ## [$CURRENT]; write them before packaging"
	target="$(host_target)"
	archive="$DIST_DIR/ownserp-$CURRENT-$target.tar.gz"
	if [[ "$DRY_RUN" -eq 1 ]]; then
		say INFO "dry run: would run tools/verify.sh, build the release binary, and write $archive"
		return 0
	fi
	"$REPO/tools/verify.sh"
	cargo build --release --locked --bin ownserp
	mkdir -p "$DIST_DIR"
	STAGE="$(mktemp -d)"
	stage="$STAGE"
	install -m 0755 target/release/ownserp "$stage/ownserp"
	cp README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE server.json "$stage/"
	mkdir -p "$stage/mcpb"
	cp mcpb/manifest.json "$stage/mcpb/manifest.json"
	tar -czf "$archive" -C "$stage" .
	sha256_of "$archive" >"$archive.sha256"
	say SUCCESS "wrote $archive"
	say SUCCESS "wrote $archive.sha256"
	if [[ "$MAKE_TAG" -eq 1 ]]; then
		git rev-parse -q --verify "refs/tags/v$CURRENT" >/dev/null && die "tag v$CURRENT already exists"
		if [[ -n "$(git config user.signingkey || true)" ]]; then
			git tag -s "v$CURRENT" -m "OwnSERP $CURRENT"
		else
			git tag -a "v$CURRENT" -m "OwnSERP $CURRENT"
		fi
		say SUCCESS "created tag v$CURRENT (not pushed)"
	fi
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
esac
