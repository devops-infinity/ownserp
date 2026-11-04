#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"
source "$REPO/tools/lib.sh"
require_bash 4.4
shopt -s inherit_errexit

PROFILE="release"
INSTALL_DIR="${OWN_SERPER_INSTALL_DIR:-$HOME/.local/bin}"
SKIP_GATE=0
RUN_DOCTOR=0
FULL_CLEAN=0
KEEP_BUILD=0
PRUNE_ONLY=0
STALE_DAYS="${OWN_SERPER_STALE_DAYS:-7}"
KEEP_INCREMENTAL=8
BUILD_DIR=""
STALE_HASHES=""

usage() {
	cat <<'USAGE'
Usage: tools/reinstall.sh [options]

Build OwnSERP and install the binary into your own bin directory.

  --debug           build the debug profile instead of release
  --dir <path>      install into this directory (default ~/.local/bin)
  --skip-gate       install without running tools/verify.sh first
  --doctor          run the installed binary's doctor command afterwards
  --full-clean      empty target/ first, so the next build starts cold
  --keep-build      leave target/ alone this run
  --stale-days <n>  drop unused build data older than this (default 7)
  --prune-only      drop stale build data and exit without building
  -h, --help        show this

Every run drops stale and orphaned build data from target/ and says how much
it reclaimed. The caches the next build reuses are kept, so a rebuild stays
fast. Use --full-clean to reclaim all of it and pay for one cold build.

Environment:
  OWN_SERPER_INSTALL_DIR   same as --dir
  OWN_SERPER_STALE_DAYS    same as --stale-days
USAGE
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--debug)
		PROFILE="debug"
		shift
		;;
	--dir)
		[[ $# -ge 2 ]] || die "--dir needs a path"
		INSTALL_DIR="$2"
		shift 2
		;;
	--skip-gate)
		SKIP_GATE=1
		shift
		;;
	--doctor)
		RUN_DOCTOR=1
		shift
		;;
	--full-clean)
		FULL_CLEAN=1
		shift
		;;
	--keep-build)
		KEEP_BUILD=1
		shift
		;;
	--stale-days)
		[[ $# -ge 2 ]] || die "--stale-days needs a number of days"
		STALE_DAYS="$2"
		shift 2
		;;
	--prune-only)
		PRUNE_ONLY=1
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

[[ "$STALE_DAYS" =~ ^[0-9]+$ ]] || die "stale days must be a whole number, got: $STALE_DAYS"
if [[ $FULL_CLEAN -eq 1 && $KEEP_BUILD -eq 1 ]]; then
	warn "--full-clean and --keep-build were both given; --full-clean wins"
fi

[[ -n "$INSTALL_DIR" ]] || die "the install directory is empty"
require_tool cargo "install Rust from https://rustup.rs"

if [[ "$(stat -f '%m' . 2>/dev/null)" =~ ^[0-9]+$ ]]; then
	mtimes() { stat -f '%Fm %N' "$@"; }
elif [[ "$(stat -c '%.9Y' . 2>/dev/null)" =~ ^[0-9] ]]; then
	mtimes() { stat -c '%.9Y %n' "$@"; }
else
	mtimes() { stat -c '%Y %n' "$@"; }
fi

human_kb() {
	awk -v kb="${1:-0}" 'BEGIN {
		split("KB MB GB TB", unit, " ")
		i = 1
		while (kb >= 1024 && i < 4) {
			kb /= 1024
			i++
		}
		if (i == 1) {
			printf "%d %s\n", kb, unit[i]
		} else {
			printf "%.1f %s\n", kb, unit[i]
		}
	}'
}

dir_kb() {
	if [[ ! -d "$1" ]]; then
		printf '0\n'
		return 0
	fi
	du -sk "$1" 2>/dev/null | awk 'NR == 1 { print $1 + 0 }' || true
}

resolve_build_dir() {
	local target="$REPO/target" repo_real
	if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
		log_line INFO "CARGO_TARGET_DIR is set, so target/ is left alone"
		return 1
	fi
	[[ -d "$target" ]] || return 1
	if [[ -L "$target" ]]; then
		warn "target is a link, so it is left alone"
		return 1
	fi
	repo_real="$(cd -- "$REPO" && pwd -P)" || return 1
	BUILD_DIR="$(cd -- "$target" && pwd -P)" || return 1
	case "$BUILD_DIR" in
	"$repo_real"/*) return 0 ;;
	esac
	warn "the build directory sits outside $REPO, so it is left alone"
	BUILD_DIR=""
	return 1
}

inside_build_dir() {
	local path="$1" real
	[[ -n "$BUILD_DIR" && "$path" == /* ]] || return 1
	[[ -d "$path" && ! -L "$path" ]] || return 1
	real="$(cd -- "$path" 2>/dev/null && pwd -P)" || return 1
	case "$real" in
	"$BUILD_DIR" | "$BUILD_DIR"/*) return 0 ;;
	esac
	return 1
}

remove_path() {
	local path="$1"
	[[ -n "$path" ]] || return 0
	if [[ "$path" != /* || "$path" == *..* ]]; then
		warn "refused to remove $path"
		return 0
	fi
	[[ -e "$path" || -L "$path" ]] || return 0
	if ! inside_build_dir "$(dirname -- "$path")"; then
		warn "refused to remove $path"
		return 0
	fi
	rm -rf -- "$path" || warn "could not remove $path"
}

newest_mtime() {
	local dir="$1" newest=0 stamp line
	while IFS= read -r line; do
		stamp="${line%% *}"
		stamp="${stamp%%.*}"
		[[ "$stamp" =~ ^[0-9]+$ ]] || continue
		if [[ $stamp -gt $newest ]]; then
			newest=$stamp
		fi
	done < <(mtimes "$dir" "$dir"/* 2>/dev/null)
	printf '%s\n' "$newest"
}

add_stale_hashes() {
	[[ -n "$1" ]] || return 0
	STALE_HASHES="$(printf '%s\n%s\n' "$STALE_HASHES" "$1")"
}

prune_stale_dirs() {
	local active="$BUILD_DIR/$PROFILE" cutoff entry touched
	cutoff=$(($(date +%s) - STALE_DAYS * 86400))
	while IFS= read -r entry; do
		if [[ "$entry" == "$active" ]]; then
			continue
		fi
		if [[ "$entry" == "$BUILD_DIR/tmp" ]]; then
			remove_path "$entry"
			continue
		fi
		touched="$(newest_mtime "$entry")"
		if [[ $touched -gt 0 && $touched -lt $cutoff ]]; then
			log_line INFO "dropping ${entry#"$BUILD_DIR"/}; nothing has touched it for $STALE_DAYS days"
			remove_path "$entry"
		fi
	done < <(find "$BUILD_DIR" -mindepth 1 -maxdepth 1 -type d 2>/dev/null)
}

prune_units() {
	local dir="$1"
	local fingerprints="$dir/.fingerprint"
	local live stamped current stale found newest rustc_id hash entry
	[[ -d "$fingerprints" ]] || return 0

	live="$(find "$fingerprints" -mindepth 1 -maxdepth 1 -type d 2>/dev/null |
		sed -n 's|.*-\([0-9a-f]\{16\}\)$|\1|p' | sort -u)" || true
	[[ -n "$live" ]] || return 0

	STALE_HASHES=""
	newest="$(mtimes "$fingerprints"/*/*.json 2>/dev/null | sort -rn | head -1)" || true
	rustc_id=""
	if [[ -n "$newest" ]]; then
		rustc_id="$(sed -n 's|.*"rustc":\([0-9]*\).*|\1|p' "${newest#* }" 2>/dev/null | head -1)" || true
	fi
	if [[ -n "$rustc_id" ]]; then
		stamped="$(grep -l -F '"rustc":' "$fingerprints"/*/*.json 2>/dev/null |
			sed -n 's|.*-\([0-9a-f]\{16\}\)/[^/]*$|\1|p' | sort -u)" || true
		current="$(grep -l -F "\"rustc\":$rustc_id," "$fingerprints"/*/*.json 2>/dev/null |
			sed -n 's|.*-\([0-9a-f]\{16\}\)/[^/]*$|\1|p' | sort -u)" || true
		if [[ -n "$stamped" && -n "$current" ]]; then
			stale="$(comm -23 <(printf '%s\n' "$stamped") <(printf '%s\n' "$current"))" || true
			if [[ -n "$stale" ]]; then
				log_line INFO "dropping build data left by an older toolchain"
			fi
			add_stale_hashes "$stale"
		fi
	fi

	if [[ -f "$REPO/Cargo.lock" && -d "$dir/deps" ]]; then
		found="$(grep -o -H -E 'registry/src/[^/ ]+/[^/ ]+/' "$dir"/deps/*.d 2>/dev/null |
			awk -v lock="$REPO/Cargo.lock" '
				BEGIN {
					name = ""
					while ((getline line < lock) > 0) {
						if (split(line, field, "\"") < 2) {
							continue
						}
						if (line ~ /^name = /) {
							name = field[2]
						} else if (line ~ /^version = / && name != "") {
							known[name "-" field[2]] = 1
							name = ""
						}
					}
				}
				{
					at = index($0, ":registry/src/")
					if (at == 0) {
						next
					}
					parts = split(substr($0, at + 1), piece, "/")
					if (!(piece[parts - 1] in known)) {
						print substr($0, 1, at - 1)
					}
				}
			' | sed -n 's|.*-\([0-9a-f]\{16\}\)\.d$|\1|p' | sort -u)" || true
		add_stale_hashes "$found"
	fi

	if [[ -d "$dir/deps" ]]; then
		found="$(find "$dir/deps" -mindepth 1 -maxdepth 1 2>/dev/null |
			sed -n 's|.*/[^/]*-\([0-9a-f]\{16\}\)\(\..*\)\{0,1\}$|\1|p' | sort -u |
			comm -23 - <(printf '%s\n' "$live"))" || true
		add_stale_hashes "$found"
	fi

	if [[ -d "$dir/build" ]]; then
		found="$(find "$dir/build" -mindepth 1 -maxdepth 1 -type d 2>/dev/null |
			sed -n 's|.*-\([0-9a-f]\{16\}\)$|\1|p' | sort -u |
			comm -23 - <(printf '%s\n' "$live"))" || true
		add_stale_hashes "$found"
	fi

	[[ -n "$STALE_HASHES" ]] || return 0
	while IFS= read -r hash; do
		[[ -n "$hash" ]] || continue
		for entry in \
			"$fingerprints"/*-"$hash" \
			"$dir"/deps/*-"$hash" \
			"$dir"/deps/*-"$hash".* \
			"$dir"/build/*-"$hash"; do
			if [[ -e "$entry" ]]; then
				remove_path "$entry"
			fi
		done
	done < <(printf '%s\n' "$STALE_HASHES" | sort -u)
	STALE_HASHES=""
}

prune_incremental() {
	local dir="$1/incremental" line entry
	local caches=()
	[[ -d "$dir" ]] || return 0
	while IFS= read -r line; do
		caches+=("$line")
	done < <(find "$dir" -mindepth 1 -maxdepth 1 -type d 2>/dev/null)
	[[ ${#caches[@]} -gt $KEEP_INCREMENTAL ]] || return 0
	while IFS= read -r line; do
		entry="${line#* }"
		remove_path "$entry"
	done < <(mtimes "${caches[@]}" 2>/dev/null | sort -rn | tail -n +$((KEEP_INCREMENTAL + 1)))
}

prune_objects() {
	local deps="$1/deps"
	inside_build_dir "$deps" || return 0
	find "$deps" -maxdepth 1 -type f -name '*.rcgu.o' -delete 2>/dev/null || true
}

clean_build_dir() {
	local before after reclaimed profile

	resolve_build_dir || return 0

	before="$(dir_kb "$BUILD_DIR")"
	log_line INFO "target/ holds $(human_kb "$before")"

	if [[ $FULL_CLEAN -eq 1 ]]; then
		cargo clean --color=never >/dev/null 2>&1 || warn "cargo clean did not finish"
	else
		if command -v cargo-sweep >/dev/null 2>&1; then
			cargo sweep --installed "$BUILD_DIR" >/dev/null 2>&1 ||
				warn "cargo-sweep did not finish; the built-in clean still ran"
		fi
		prune_stale_dirs
		while IFS= read -r profile; do
			prune_units "$profile"
			prune_incremental "$profile"
			prune_objects "$profile"
		done < <(find "$BUILD_DIR" -maxdepth 3 -type d -name '.fingerprint' 2>/dev/null |
			sed 's|/\.fingerprint$||')
	fi

	after="$(dir_kb "$BUILD_DIR")"
	reclaimed=$((before - after))
	if [[ $after -eq 0 ]]; then
		log_line SUCCESS "cleaned $(human_kb "$before"); target/ is empty"
	elif [[ $reclaimed -gt 0 ]]; then
		log_line SUCCESS "cleaned $(human_kb "$reclaimed"); target/ is now $(human_kb "$after")"
	else
		log_line INFO "nothing stale to clean; target/ stays at $(human_kb "$after")"
	fi
}

if [[ $FULL_CLEAN -eq 1 || $KEEP_BUILD -eq 0 ]]; then
	clean_build_dir
else
	log_line INFO "leaving target/ alone"
fi

if [[ $PRUNE_ONLY -eq 1 ]]; then
	exit 0
fi

if [[ "$SKIP_GATE" -eq 0 ]]; then
	log_line INFO "running the verification gate"
	"$REPO/tools/verify.sh"
fi

log_line INFO "building the $PROFILE binary"
if [[ "$PROFILE" == "release" ]]; then
	cargo build --release --locked --bin ownserp
	BINARY="$REPO/target/release/ownserp"
else
	cargo build --locked --bin ownserp
	BINARY="$REPO/target/debug/ownserp"
fi

[[ -x "$BINARY" ]] || die "the build did not produce $BINARY"
mkdir -p "$INSTALL_DIR"
INSTALL_TMP="$(mktemp "$INSTALL_DIR/.ownserp.new.XXXXXX")"
trap 'rm -f -- "${INSTALL_TMP:-}"' EXIT
install -m 0755 "$BINARY" "$INSTALL_TMP"
mv -f -- "$INSTALL_TMP" "$INSTALL_DIR/ownserp"
log_line SUCCESS "installed $("$INSTALL_DIR/ownserp" --version) to $INSTALL_DIR/ownserp"

case ":$PATH:" in
*":$INSTALL_DIR:"*) ;;
*) warn "$INSTALL_DIR is not on PATH; add it to your shell profile" ;;
esac

if [[ "$RUN_DOCTOR" -eq 1 ]]; then
	"$INSTALL_DIR/ownserp" doctor
fi
