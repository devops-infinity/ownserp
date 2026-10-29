#!/usr/bin/env bash
set -Eeuo pipefail

REPO="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="${1:-http://127.0.0.1:8777}"
source "$REPO/tools/lib.sh"
require_bash 4.4
require_tool curl "install curl to run the smoke test"
require_tool python3 "install python3 to run the smoke test"

TOKEN_HEADER=()
if [[ -n "${OWN_SERPER_BEARER_TOKENS:-}" ]]; then
	TOKEN_HEADER=(-H "Authorization: Bearer ${OWN_SERPER_BEARER_TOKENS%%,*}")
fi
PROTOCOL_VERSION="${OWN_SERPER_PROTOCOL_VERSION:-2025-11-25}"

check() {
	local name="$1"
	shift
	if "$@" >/dev/null 2>&1; then
		log_line SUCCESS "$name"
	else
		die "$name failed against $BASE"
	fi
}

check "healthz" curl -fsS "$BASE/healthz"
check "readyz" curl -fsS "${TOKEN_HEADER[@]}" "$BASE/readyz"
check "metrics" curl -fsS "${TOKEN_HEADER[@]}" "$BASE/metrics"

initialize_body="$(mktemp)"
tools_body="$(mktemp)"
trap 'rm -f "$initialize_body" "$tools_body"' EXIT

curl -fsS -X POST "$BASE/" \
	"${TOKEN_HEADER[@]}" \
	-H 'content-type: application/json' \
	-H 'accept: application/json, text/event-stream' \
	-d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"$PROTOCOL_VERSION\",\"capabilities\":{},\"clientInfo\":{\"name\":\"ownserp-smoke\",\"version\":\"0\"}}}" \
	>"$initialize_body"

curl -fsS -X POST "$BASE/" \
	"${TOKEN_HEADER[@]}" \
	-H 'content-type: application/json' \
	-H 'accept: application/json, text/event-stream' \
	-H "MCP-Protocol-Version: $PROTOCOL_VERSION" \
	-d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
	>"$tools_body"

python3 - "$initialize_body" "$tools_body" <<'PYJSON'
import json
import sys

with open(sys.argv[1]) as handle:
    handshake = json.load(handle)
server = handshake.get("result", {}).get("serverInfo", {})
if not server.get("version"):
    raise SystemExit("initialize answered without a server version")

with open(sys.argv[2]) as handle:
    listing = json.load(handle)
tools = listing.get("result", {}).get("tools", [])
if not tools:
    raise SystemExit("tools/list answered with no tools")
print(f"server {server['version']}, {len(tools)} tools")
PYJSON
log_line SUCCESS "mcp handshake and tools/list"
log_line SUCCESS "smoke test passed against $BASE"
