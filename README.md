# OwnSERP

<!-- mcp-name: io.github.devops-infinity/ownserp -->

An MCP server for the Serper.dev search API, with per-call credit accounting.

## Status

Stable at 1.0. Every release ships from a tagged commit after the full local gate passes. From 1.0, the command surface, the configuration schema, the ledger format, and the tool result shape follow semantic versioning: a breaking change to any of them takes a major version, a new tool, flag, or setting takes a minor version, and a fix takes a patch. `CHANGELOG.md` records every change.

## Contents

- [Overview](#overview)
- [Requirements](#requirements)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Connect a client](#connect-a-client)
- [API key](#api-key)
- [Tools](#tools)
- [Response shape and field presets](#response-shape-and-field-presets)
- [Cost accounting and the ledger](#cost-accounting-and-the-ledger)
- [Configuration](#configuration)
- [HTTP deployment](#http-deployment)
- [Security](#security)
- [Troubleshooting](#troubleshooting)
- [Development](#development)
- [Distribution](#distribution)
- [Support](#support)
- [Contributing](#contributing)
- [License](#license)

## Overview

OwnSERP exposes the Serper.dev search endpoints as MCP (Model Context Protocol) tools. A model can search the web, images, news, places, maps, videos, shopping, scholar, patents, reviews, and lens, extract a page, resolve a location name, and read its own spend, all through one local server.

It is built for agents that spend metered credits. Every tool description states its price, every result carries a cost object, and a local ledger records each call. Before a call spends anything, the server checks a per-call ceiling and an optional session budget and refuses the call when either would be passed. Results are shaped for a model rather than for a browser: invisible characters are stripped, long text is capped and marked, absent fields are omitted, and key order is fixed.

The server is one binary and needs no language runtime. It speaks MCP revision 2026-07-28 and answers clients on 2025-11-25. It runs over stdio by default, or over Streamable HTTP with a bearer token gate.

## Requirements

- To run a published binary: no runtime, no toolchain. Prebuilt binaries cover macOS, Linux, and Windows.
- To build from source: Rust 1.89 or newer, pinned to 1.99.1 in `rust-toolchain.toml`.
- A Serper.dev account and API key from https://serper.dev/api-keys.
- A client that speaks MCP over stdio, or any HTTP client for the Streamable HTTP transport.

## Installation

Every install path uses a prebuilt binary: no Rust toolchain and no compile step.

Release archives cover macOS ARM64 and x86_64, Linux x86_64 with glibc, Linux ARM64 with glibc, Linux x86_64 and ARM64 with static musl, and Windows x86_64.

### npm

```sh
npx @devops-infinity/ownserp serve
```

The package downloads the archive for the current platform on install, checks it against the release checksum, and runs the binary. Node 18 or newer.

### Homebrew

```sh
brew install devops-infinity/tap/ownserp
```

macOS and Linux both install the release binary.

### Shell installer

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/devops-infinity/ownserp/releases/latest/download/ownserp-installer.sh | sh
```

On Windows, in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/devops-infinity/ownserp/releases/latest/download/ownserp-installer.ps1 | iex"
```

### Release archive

```sh
VERSION=1.2.0
BASE="https://github.com/devops-infinity/ownserp/releases/download/v$VERSION"
TARGET=x86_64-unknown-linux-gnu
curl -LO "$BASE/ownserp-$TARGET.tar.gz"
curl -LO "$BASE/ownserp-$TARGET.tar.gz.sha256"
sha256sum -c "ownserp-$TARGET.tar.gz.sha256"
tar -xzf "ownserp-$TARGET.tar.gz"
install -m 0755 "ownserp-$TARGET/ownserp" ~/.local/bin/ownserp
```

Set `TARGET` to `aarch64-apple-darwin` or `x86_64-apple-darwin` on macOS, `aarch64-unknown-linux-gnu` or `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` on those platforms, and use the `.zip` file with `x86_64-pc-windows-gnu` on Windows.

The binary prints its shell completions and manual page on demand with `ownserp completions <shell>` and `ownserp man`. Install them where your shell and manual reader look, under `~/.local/share/bash-completion/completions/`, a directory on `fpath`, `~/.config/fish/completions/`, and `~/.local/share/man/man1/`.

Each release signs `sha256.sum` with minisign. To check a download, verify the signature against the public key in this repository, then check the archive against the file:

```sh
minisign -Vm sha256.sum -P RWSPyQWAsXwP5vjgAFI/PAAtbFAd5tA4BlhyFrT+MCPNmE2J+jgT26kC
grep "ownserp-x86_64-unknown-linux-gnu.tar.gz" sha256.sum | sha256sum --check
```

### Container

```sh
docker run --rm -p 127.0.0.1:8777:8777 \
  -e SERPER_API_KEY -e OWN_SERPER_BEARER_TOKENS \
  -e OWN_SERPER_ALLOWED_HOSTS=127.0.0.1,localhost \
  ghcr.io/devops-infinity/ownserp:latest
```

The image serves Streamable HTTP and refuses a public bind without a bearer token, so both variables are required. `OWN_SERPER_ALLOWED_HOSTS` must name every Host a client sends, because the image binds `0.0.0.0` and the allowlist derived from that address matches no real Host; add the public hostname, or the name the proxy sends, to the same list. The package is public, so `docker pull` needs no login.

### Windows

Use the PowerShell installer above, or download `ownserp-x86_64-pc-windows-gnu.zip` from the release page, extract `ownserp.exe`, and put the folder on your PATH. A client configuration that cannot resolve the name uses the absolute path to `ownserp.exe`.

### MCPB bundle

Download `ownserp-<version>.mcpb` from the release page and open it in a client that supports MCP Bundles. The bundle asks for the API key and stores it in the OS keychain.

## Quick start

```sh
export SERPER_API_KEY="your key"
npx @devops-infinity/ownserp doctor
npx @devops-infinity/ownserp serve
```

The first command sets the key for the shell. The second prints the resolved settings, the key source with the key masked, and the answer from both Serper hosts. The third starts the server on stdio, which is the command an MCP client launches, so it keeps running until the client stops it.

To connect a client, add the server as shown in the next section. This repository carries a project `.mcp.json`, so Claude Code picks it up inside this checkout.

## Connect a client

Every client runs `ownserp serve` as a stdio process and passes `SERPER_API_KEY` in the environment. When a desktop client cannot resolve `ownserp` on its PATH, use the absolute path printed by `command -v ownserp`.

### Claude Code

```sh
claude mcp add --scope project ownserp -- ownserp serve
```

Or commit a `.mcp.json` at the project root:

```json
{
  "mcpServers": {
    "ownserp": {
      "command": "ownserp",
      "args": ["serve"]
    }
  }
}
```

### Claude Desktop

Add this to `claude_desktop_config.json`, at `~/Library/Application Support/Claude/` on macOS or `%APPDATA%\Claude\` on Windows. Use the absolute path to the installed binary, because a desktop application does not inherit the shell PATH:

```json
{
  "mcpServers": {
    "ownserp": {
      "command": "/absolute/path/to/ownserp",
      "args": ["serve"],
      "env": { "SERPER_API_KEY": "your key" }
    }
  }
}
```

The npm package installs the prebuilt binary for the current platform. Point `command` at `ownserp` when the client inherits your PATH, or at the absolute path `command -v ownserp` prints when it does not.

### OpenAI Codex CLI

In `~/.codex/config.toml`, or a project `.codex/config.toml`:

```toml
[mcp_servers.ownserp]
command = "ownserp"
args = ["serve"]
env_vars = ["SERPER_API_KEY"]
```

`env_vars` passes the variable from the environment, so the key never enters a command line.

### Command Code

```sh
commandcode mcp add-json --scope project ownserp '{"type":"stdio","command":"ownserp","args":["serve"]}'
```

### Cursor

In `~/.cursor/mcp.json`, or `.cursor/mcp.json` for one project:

```json
{
  "mcpServers": {
    "ownserp": {
      "command": "ownserp",
      "args": ["serve"],
      "env": { "SERPER_API_KEY": "your key" }
    }
  }
}
```

### Windsurf (Cascade)

In `~/.config/devin/mcp_config.json` on Linux and macOS, or `%APPDATA%\devin\mcp_config.json` on Windows:

```json
{
  "mcpServers": {
    "ownserp": {
      "command": "ownserp",
      "args": ["serve"],
      "env": { "SERPER_API_KEY": "your key" }
    }
  }
}
```

### VS Code

Run `MCP: Add Server` from the Command Palette, or add `.vscode/mcp.json` to the workspace:

```json
{
  "servers": {
    "ownserp": {
      "type": "stdio",
      "command": "ownserp",
      "args": ["serve"],
      "env": { "SERPER_API_KEY": "your key" }
    }
  }
}
```

### Zed

In `settings.json`:

```json
{
  "context_servers": {
    "ownserp": {
      "command": "ownserp",
      "args": ["serve"],
      "env": { "SERPER_API_KEY": "your key" }
    }
  }
}
```

### Other MCP clients

Use stdio with command `ownserp` and argument `serve`, and set `SERPER_API_KEY` in the client's environment block or in the shell the client inherits.

## API key

Three sources work, and the process environment wins over the keychain.

- Environment variable, in the shell or in the client's env block:

```sh
export SERPER_API_KEY="your key"
```

- OS keychain, read from stdin so the key does not enter shell history:

```sh
printf '%s' "$SERPER_API_KEY" | ownserp config set-api-key
```

- Client JSON config, where the client injects the env block it holds.

The key is never read from a command line argument, never written to the profile file, and never printed unmasked. On Linux the keychain store is Secret Service, so a desktop session bus must be running; on a headless server, set `SERPER_API_KEY` instead. Use one named key per project or agent at https://serper.dev/api-keys, because a key cannot be scoped to a smaller spend.

## Tools

Every tool description in the tool list states the price, so a model can budget before spending.

- `serper_search`, 1 credit. Web results with knowledge graph, answer box, and related searches.
- `serper_images`, 1 credit, 2 when `num` is above 10. Image results with dimensions and source.
- `serper_news`, 1 credit. Dated news results.
- `serper_places`, 1 credit. Local businesses filtered by a Serper location name.
- `serper_maps`, 3 credits. Places anchored to a map viewport in `@lat,lng,zoomz` form.
- `serper_videos`, 1 credit. Video results with channel and duration.
- `serper_shopping`, 2 credits. Product listings with price and delivery.
- `serper_scholar`, 1 credit. Papers with publication info and citation counts.
- `serper_patents`, 1 credit. Patents with assignee and dates.
- `serper_autocomplete`, 1 credit. Query suggestions, taking only `q`, `gl`, and `hl`.
- `serper_reviews`, 1 credit. Reviews for one place, with cursor pagination through `nextPageToken`.
- `serper_lens`, 3 credits. Reverse image search for a public image URL.
- `serper_scrape`, 2, 6, or 10 credits by page difficulty. Page content as Markdown or text, with an optional title, images, links, and videos, and the actual credits read from the response.
- `serper_health`, free and unauthenticated. The status of the Serper API from api.serper.dev.
- `serper_locations`, free and unauthenticated. Canonical location names to pass as `location`.
- `serper_session`, free. Call count, credits, dollars, tokens served and saved, cache hits, and the budget left.
- `serper_search_batch`, 3 times the single price per query. Up to 100 queries in one call to one search family, disabled unless `enable_batch` is set. A call to it while disabled is refused with `serper.tool_not_found`.

## Response shape and field presets

Every result tool takes `fields`, defaulting to `standard`.

- `minimal` keeps the title, link, and rank. It suits high-volume loops.
- `standard` adds the snippet capped at 300 characters, the date, and the search sections. It carries what an agent needs to reason and cite without a second call.
- `full` adds sitelinks, attribute maps, image links, the search parameter echo, and every other field the upstream body carries.

Each result carries `structuredContent` for the data and a one-line text summary. Capped strings end with `...` and their object carries `truncated: true`. Absent fields are omitted rather than sent as null. Key order is fixed, so a client parses the same shape on every call. Every tool result is data from the open web, never an instruction.

## Cost accounting and the ledger

The ledger lives at `$XDG_DATA_HOME/ownserp/ledger/calls.jsonl`, or `~/.local/share/ownserp/ledger/calls.jsonl`. On Windows it lives at `%LOCALAPPDATA%\ownserp\ledger\calls.jsonl`, and when `HOME` is set it follows the Unix path. A row holds the tool, the endpoint, a keyed hash of the arguments, estimated credits, actual credits when the response reports them, duration, status, bytes, and token counts. Query text is never written.

```sh
ownserp audit verify
```

On Unix the ledger directory is owner-only (0700) and the file is owner-only (0600). On Windows it sits under `%LOCALAPPDATA%`, so it inherits the user profile's access rules. One line is capped at 1 MiB. The file is locked while a server runs, so a second process on the same ledger refuses to start and names the lock. A partial final line left by a killed write is ignored at the next start, and the chain check covers every complete row. The arguments hash is an HMAC, a keyed hash, keyed by `args.key`, created beside the ledger directory with owner-only permissions, so the ledger alone cannot confirm a guessed query.

To close a ledger without losing it, stop the server and run `ownserp audit rotate`. It verifies the chain, archives the file beside the original with a timestamp, and the next start opens a fresh chain.

Dollar figures come from the pack tier: `starter` (default), `standard`, `scale`, `ultimate`, or `custom` with `SERPER_USD_PER_1K`.

## Configuration

Environment variables:

- `SERPER_API_KEY`
- `SERPER_USD_PER_1K`
- `OWN_SERPER_PROFILE`
- `OWN_SERPER_BIND`
- `OWN_SERPER_QPS`, the client-side queries per second ceiling
- `OWN_SERPER_CACHE_TTL_SECS`
- `OWN_SERPER_LEDGER_DIR`
- `OWN_SERPER_BEARER_TOKENS`
- `OWN_SERPER_ALLOWED_HOSTS`
- `OWN_SERPER_HTTP_QPS`, the global inbound request ceiling in HTTP mode, off when unset or 0
- `OWN_SERPER_LOG_FORMAT`, set to `json` for one JSON object per log line

Profile file at `$XDG_CONFIG_HOME/ownserp/profile.toml`, or `~/.config/ownserp/profile.toml`. On Windows it is `%APPDATA%\ownserp\profile.toml` when `HOME` is unset:

```toml
version = 1
pack = "starter"
usd_per_1k = 1.0
qps = 50
cache_ttl_secs = 0
cache_max_bytes = 16777216
ledger_dir = "/absolute/path/to/ownserp-ledger"
per_call_credit_ceiling = 10
session_budget_credits = 500
enable_batch = false
http = false
bind = "127.0.0.1:8777"
request_timeout_secs = 30
http_qps = 0
```

`version` is the profile schema version. It is 1 today, and a profile that names a newer version is refused by name.

`ledger_dir` takes an absolute path. The file is a TOML document, so a `~` or a variable is stored literally rather than expanded.

`session_budget_credits` counts every credit written to the current ledger chain, across restarts, and `ownserp audit rotate` starts a fresh count. `allowed_hosts` lists the Host names HTTP mode accepts; leave it unset to derive them from the bind address.

A custom pack with no `usd_per_1k` reports credits with no dollar figure, because the rate is unknown.

The cache is off by default. `cache_ttl_secs` above zero turns it on; a hit answers with `cacheHit: true` and charges nothing. A call with `tbs` set skips the cache.

`ownserp doctor` prints the resolved settings, masks the key, names where the key came from, and probes both hosts. `ownserp config show` prints the key source, the masked key, the credit pack, the ledger directory, and the profile path.

## HTTP deployment

```sh
OWN_SERPER_BEARER_TOKENS="a random value of at least 32 characters" \
  ownserp serve --http --bind 127.0.0.1:8777
```

`/healthz` is open. `/readyz` and `/metrics` require the bearer token whenever `OWN_SERPER_BEARER_TOKENS` is set, and every other path is the MCP endpoint. The server refuses a non-loopback bind without a token and refuses a token shorter than 32 characters. It checks the Host header against the allowlist on every protected route, caps a request body at 1 MiB and gives the body 30 seconds to arrive, accepts at most 256 connections, rate limits before it checks the token, holds at most 128 requests at once, gives each request a 120 second deadline, rejects a request that carries an Origin header, and marks every response `x-content-type-options: nosniff` with `cache-control: no-store`. Every bearer token on one server shares that server's ledger, session budget, and cache. Behind a proxy, list the names it sends in `OWN_SERPER_ALLOWED_HOSTS` and let the proxy enforce its own header and idle timeouts.

## Security

The server holds one API key, sends queries only to Serper over TLS, and sends no telemetry. Results are treated as untrusted text. [SECURITY.md](SECURITY.md) states the reporting process, and the [threat model](docs/threat-model.md) states the assets, trust boundaries, controls, and residual risks.

## Troubleshooting

Each error carries a stable code, a message, a remedy, and, for a network failure, up to three sanitized causes that name a DNS, TLS, or connection problem. `ownserp` exits 0 on success, 1 on a runtime failure, and 2 on a usage error. `ownserp doctor` exits 1 when no key is configured or when the Serper health probe fails; a balance the account endpoint does not report is printed but is not a failure. The codes and their fixes:

- `serper.auth_missing`. No key is configured. Set `SERPER_API_KEY`, or run `ownserp config set-api-key`.
- `serper.auth_failed`. Serper rejected the key. Check it at https://serper.dev/api-keys.
- `serper.rate_limited`. The upstream rate limit or the credit balance stopped the call. Serper publishes no retry header, and the server does not retry a 429.
- `serper.budget_refused`. The per-call ceiling or the session budget would be passed. Raise it in the profile, or lower `num`.
- `serper.bad_request`. An argument failed validation. The message names the field.
- `serper.tool_not_found`. The tool is not in this configuration's list. Call `tools/list`, and set `enable_batch` for the batch tool.
- `serper.config_error`. A setting is malformed. `ownserp doctor` shows each resolved value and its source.
- `serper.keychain_error`. The OS keychain is unavailable. Use `SERPER_API_KEY` on a headless host.
- `serper.upstream_failed`. Serper or the network failed after retries. Check https://serper.betteruptime.com.
- `serper.cancelled`. The client cancelled the request.
- `serper.internal_error`. An unexpected local failure. Retry once, then open an issue with the message.

## Development

This path needs Rust 1.89 or newer, pinned to 1.99.1 in `rust-toolchain.toml`.

```sh
cargo install --path crates/ownserp --locked
tools/verify.sh
```

`tools/verify.sh` runs the whole gate locally, with no hosted pipeline: formatting, check, clippy with warnings denied, tests, doc tests, docs, advisories, dependency policy, unused dependencies, the 80 percent line coverage floor, and a source-only secret scan. A check whose tool is missing is reported as skipped, and `OWN_SERPER_STRICT_GATE=1` turns any skip into a failure; the release runs that way.

The local lifecycle:

- `tools/reinstall.sh` builds and installs the binary into `~/.local/bin`.
- `tools/release.sh --prepare` bumps the version; `--package` builds the archive and checksum.
- `tools/publish-npm.sh` publishes the npm package; `tools/npm-auth-setup.sh` stores the token that makes it OTP-free.
- `tools/registry-auth.sh` refreshes the GitHub, npm, crates.io, MCP Registry, and ghcr sessions from the environment variables already in your shell, so no registry asks for a login during a release.
- `tools/install-hooks.sh` points git at the local hooks: a secret scan before each commit and the full gate before each push.
- [RELEASING.md](RELEASING.md) is the release runbook.
- [docs/benchmark.md](docs/benchmark.md) compares the live tool surface with the Python Serper MCP server and records the measured result.

## Distribution

- crates.io: `ownserp` and `ownserp-core`.
- npm: `@devops-infinity/ownserp`.
- Homebrew tap: `devops-infinity/tap`.
- MCP Registry: `io.github.devops-infinity/ownserp`.
- GitHub releases: archives for macOS ARM64 and x86_64, Linux x86_64 and ARM64 with glibc, Linux x86_64 and ARM64 with static musl, and Windows x86_64, each with a checksum, plus a shell installer, a PowerShell installer, and a Linux MCPB bundle.
- Container image: `ghcr.io/devops-infinity/ownserp`, public.
- Manifests in this repository: `server.json`, `mcpb/manifest.json`, and `Dockerfile`.

## Support

Open an issue at https://github.com/devops-infinity/ownserp/issues for a question, a bug, or a feature request. Report a security problem through the private channel in `SECURITY.md`, never in an issue.

## Contributing

Issues and pull requests are welcome at https://github.com/devops-infinity/ownserp. Run `tools/verify.sh` before opening a pull request; a change lands only when the gate passes. Read `RELEASING.md` before cutting a release.

Results come from Google through Serper. Downstream users keep the attribution duty Serper's terms place on them.

## License

MIT OR Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
