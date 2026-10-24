# ownserp

An MCP server, written in Rust, for the Serper.dev search API. It turns each Serper endpoint into an MCP tool, reports the credit cost of every call beside the data, and keeps a local ledger you can verify.

## What it does

- Speaks MCP revision 2026-07-28 and still answers clients on 2025-11-25.
- Runs over stdio by default, or over Streamable HTTP with a bearer token gate.
- Pins an input schema and an output schema on every tool, so a client sees the full contract.
- Puts a `_cost` object on every result with the credits for that call, the session total, and the dollar equivalent.
- Writes one row per call to an append-only, hash-chained ledger, and checks the chain on every start.
- Treats each result string as untrusted web data. Invisible characters are stripped, long text is capped and marked `truncated`, and absent fields are omitted instead of sent as null.
- Refuses a call before it spends anything when it passes the per-call credit ceiling or the session budget.

## Install

You need Rust 1.89 or newer. The repository pins 1.99.1 in `rust-toolchain.toml`.

```sh
cargo install ownserp
```

From this checkout instead:

```sh
cargo install --path crates/ownserp --locked
```

Check the resolved settings and both Serper hosts:

```sh
ownserp doctor
```

## Set the API key

Three sources work, and the process environment wins over the keychain.

- Environment variable, set in your shell or in the client's own env block:

```sh
export SERPER_API_KEY="your key"
```

- OS keychain, read from stdin so the key never sits in your shell history:

```sh
printf '%s' "$SERPER_API_KEY" | ownserp config set-api-key
```

- Client JSON config, where the client injects the env block it already holds:

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

The key is never read from a command line argument, never written to the profile file, and never printed unmasked. `docs/threat-model.md` states what each boundary protects and what stays as residual risk.

On Linux the keychain store is Secret Service, so a desktop session bus must be running. On a headless server, set `SERPER_API_KEY` instead. The process environment always wins over the keychain, which is also why a value injected by a client's env block takes effect without touching the keychain.

## Run over HTTP

```sh
OWN_SERPER_BEARER_TOKENS="token-for-this-deployment" \
  ownserp serve --http --bind 127.0.0.1:8777
```

`/healthz` is open, `/readyz` and `/metrics` need the bearer token, and every other path is the MCP endpoint. The server refuses to bind a non-loopback address without at least one token, refuses any token shorter than 32 characters, checks the Host header against the allowlist on every protected route, caps a request body at 1 MiB, and marks every response `x-content-type-options: nosniff` with `cache-control: no-store`. Behind a proxy, list the names it will send in `OWN_SERPER_ALLOWED_HOSTS`.

## Tools

Each tool costs the credits named here, and its description in the tool list repeats the figure so a model can budget before spending.

- `serper_search`, 1 credit. Web results with knowledge graph, answer box, and related searches.
- `serper_images`, 1 credit, 2 when `num` is above 10. Image results.
- `serper_news`, 1 credit. Dated news results.
- `serper_places`, 1 credit. Local businesses filtered by a Serper location name.
- `serper_maps`, 3 credits. Places anchored to a map viewport in `@lat,lng,zoomz` form.
- `serper_videos`, 1 credit. Video results with channel and duration.
- `serper_shopping`, 2 credits. Product listings with price and delivery.
- `serper_scholar`, 1 credit. Papers with publication info and citation counts.
- `serper_patents`, 1 credit. Patents with assignee and dates.
- `serper_autocomplete`, 1 credit. Query suggestions, taking only `q`, `gl`, and `hl`.
- `serper_reviews`, 1 credit. Reviews for one place, with cursor pagination.
- `serper_lens`, 3 credits. Reverse image search for a public image URL.
- `serper_scrape`, 2, 6, or 10 credits by page difficulty. Page content as Markdown or text.
- `serper_locations`, free and unauthenticated. Canonical location names to pass as `location`.
- `serper_session`, free. Call count, credits, dollars, tokens served and saved, cache hits, and budget left.
- `serper_search_batch`, 3 times the single price per query. Up to 100 queries in one call, disabled unless `enable_batch` is set. A call to it while disabled is refused, not merely hidden from the tool list.

## Field presets

Every result tool takes `fields`, defaulting to `standard`.

- `minimal` keeps the title, link, and rank. Use it in high volume loops.
- `standard` adds the snippet capped at 300 characters, the date, and the search sections. This is what an agent needs to reason and cite without a second call.
- `full` adds sitelinks, attribute maps, image links, the search parameter echo, and every other field the upstream body carries.

Capped strings end with `...` and their object carries `truncated: true`. Key order is fixed, so a client parses the same shape on every call.

## Credit accounting

The ledger lives in `$XDG_DATA_HOME/ownserp/ledger/calls.jsonl`, or `~/.local/share/ownserp/ledger/calls.jsonl`. Each row holds the tool, the endpoint, a hash of the arguments, estimated credits, actual credits when the response reports them, duration, status, bytes, and token counts. Raw query text is never written.

```sh
ownserp audit verify
```

On Unix the ledger directory is owner-only (0700) and the file is owner-only (0600). A single line is capped at 1 MiB, so a damaged file cannot exhaust memory at startup. The file is locked while a server runs, so a second process on the same ledger refuses to start with a clear message. `audit rotate` also refuses while the lock is held. A partial final line left by a killed write is ignored on the next start, and the chain check covers every complete row.

The arguments hash is an HMAC keyed by `args.key`, created once beside the ledger directory with owner-only permissions on Unix. Without that key the ledger cannot confirm whether a guessed query was made. To start a new file without losing the old one, stop the server and run `ownserp audit rotate`. It verifies the chain, archives the file beside the original with a timestamp, and the next start writes a fresh chain.

Dollar figures come from the pack tier: `starter` (default), `standard`, `scale`, `ultimate`, or `custom` with `SERPER_USD_PER_1K`.

## Configuration

Environment variables:

- `SERPER_API_KEY`
- `SERPER_USD_PER_1K`
- `OWN_SERPER_PROFILE`
- `OWN_SERPER_BIND`
- `OWN_SERPER_QPS`
- `OWN_SERPER_CACHE_TTL_SECS`
- `OWN_SERPER_LEDGER_DIR`
- `OWN_SERPER_BEARER_TOKENS`
- `OWN_SERPER_ALLOWED_HOSTS`
- `OWN_SERPER_HTTP_QPS`, the global inbound request ceiling in HTTP mode, off when unset or 0
- `OWN_SERPER_LOG_FORMAT`, set to `json` for one JSON object per log line

Profile file at `$XDG_CONFIG_HOME/ownserp/profile.toml`, or `~/.config/ownserp/profile.toml`:

```toml
pack = "starter"
usd_per_1k = 1.0
qps = 50
cache_ttl_secs = 0
cache_max_bytes = 16777216
ledger_dir = "/home/you/.local/share/ownserp/ledger"
per_call_credit_ceiling = 10
session_budget_credits = 500
enable_batch = false
http = false
bind = "127.0.0.1:8777"
request_timeout_secs = 30
http_qps = 0
```

The cache is off by default. `cache_ttl_secs` above zero turns it on, and a hit answers with `cacheHit: true` and charges nothing. Calls with `tbs` set skip the cache.

## Development

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo test --workspace --all-features --locked --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo machete
cargo deny check
cargo llvm-cov --workspace --all-features --fail-under-lines 80 --summary-only
```

Line coverage measured on 2026-10-24 is 80.2 percent, and `tools/verify.sh` fails the gate below 80 percent line coverage through `cargo llvm-cov --fail-under-lines 80`, so the figure cannot silently fall.

The tests use recorded payloads and a local mock server. No test calls Serper, so the suite needs no key and spends no credits.

The advisory scan is:

```sh
cargo audit --deny warnings \
  --ignore RUSTSEC-2026-0370 \
  --ignore RUSTSEC-2026-0371 \
  --ignore RUSTSEC-2026-0372
```

Those three flags are unmaintained notices on `zvariant`, `zbus_names`, and `zvariant_derive`, which reach the tree only through the Secret Service keychain store. `deny.toml` carries the same entries with a reason and a review date. `cargo-deny` is not installed on this machine, so `deny.toml` has not been run yet.

## Local lifecycle

- `tools/verify.sh` runs the whole gate: formatting, check, clippy, tests, docs, advisories, policy, unused dependencies, and a secret scan.
- `tools/reinstall.sh` builds and installs the binary into `~/.local/bin`.
- `tools/release.sh --prepare` bumps the version, and `--package` builds the archive and checksum.
- `tools/install-hooks.sh` points git at the local hooks: a secret scan before each commit and the full gate before each push.
- `RELEASING.md` is the runbook for cutting a release.

There is no GitHub workflow and no hosted pipeline. Every check and every release step runs on this machine.

## Distribution files

- `server.json` is the MCP Registry manifest for `io.github.devops-infinity/ownserp`.
- `mcpb/manifest.json` is the bundle manifest for a binary server with a keychain-backed key.
- `Dockerfile` builds a small image that serves Streamable HTTP, with both base images pinned by digest.

Publishing to crates.io, npm, Homebrew, the MCP Registry, and ghcr is a maintainer step and is not done by this repository.

## Toolchain policy

The crate pins Rust 1.99.1 and declares an MSRV of 1.89, which is above the floor of every dependency. The MSRV is raised only when a dependency or a language feature needs it, and the change is recorded in `CHANGELOG.md` and ships in a minor release.

The edition is 2024. When a new edition ships, the upgrade runs in one change: `cargo update`, `cargo fix --edition`, the `edition` field bump, `cargo fmt`, then the full gate from the Development section above. No edition is kept alive beside the new one.

mcp-name: io.github.devops-infinity/ownserp

## License

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.

Results come from Google through Serper. Downstream users keep the attribution duty Serper's terms place on them.
