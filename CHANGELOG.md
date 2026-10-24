# Changelog

All notable changes to this project are recorded here. The format follows Keep a Changelog 2.0.0, and the project uses semantic versioning.

## Policy

The minimum supported Rust version is 1.89. Raising it is recorded in this file and ships in a minor release.

## [0.1.2] - 2026-10-24

### Fixed

- The binary crate description now starts with OwnSERP, so crates.io shows the product name.
- The secret scan reads source only, after a rustdoc search index matched a rule by accident on every release build.

### Added

- An npm loader, `@devops-infinity/ownserp`, that downloads the pinned Linux x86_64 binary, checks it against the published SHA-256, and runs it.
- A project `.mcp.json` that starts the server with the key from the client environment.

### Changed

- The release script stamps the npm package and the version the loader downloads.
- The manifest names both the cargo and npm packages for the MCP Registry.

## [0.1.1] - 2026-10-24

### Changed

- The product label reads OwnSERP wherever a person or a client sees it: the CLI name and version line, the MCP server identity, the README, the release title, the bundle display name, the image labels, and both crate descriptions. Package and registry identifiers stay lowercase.
- The keyword `model-context-protocol` became `search-api`, because crates.io rejects a keyword longer than 20 characters.
- Both packages carry the workspace README, so crates.io renders it.
- Settings resolves through an injectable environment source, so the test suite no longer reads the developer shell.
- The verification gate enforces the 80 percent line coverage floor.

### Added

- Repository hooks: a staged secret scan before each commit and the full gate before each push.
- An installer, a release script, and a release runbook that run entirely on this machine, with no hosted pipeline.

## [0.1.0] - 2026-10-24

### Added

- MCP server over stdio and Streamable HTTP, speaking 2026-07-28 and 2025-11-25.
- Fifteen default tools covering search, images, news, places, maps, videos, shopping, scholar, patents, autocomplete, reviews, lens, scrape, locations, and session statistics.
- `serper_search_batch`, behind the `enable_batch` setting, with up to 100 queries per call at three times the single price.
- Field presets `minimal`, `standard`, and `full`, with compact serialization and a 256 KiB result budget.
- Per-call cost object and a hash-chained ledger, with `ownserp audit verify`.
- Per-call credit ceiling, session budget, client-side QPS limit, and retry with backoff for transient failures.
- Response cache with a TTL and a byte cap, off by default.
- Structured errors with stable `serper.*` codes and a remedy line.
- Binary CLI with `serve`, `doctor`, `config`, and `audit` commands.
- Registry manifest, bundle manifest, and Dockerfile.
- Local lifecycle tooling: a verification gate, an installer, a release script, and repository hooks. No hosted pipeline.
- An exclusive ledger lock, owner-only ledger permissions, a per-line size cap, and a keyed arguments hash.
- Unknown and disabled tool calls are refused, and credits are reserved at the worst known price before a call.
- Retries pass through the local rate limiter, and upstream error text is sanitized and capped.
- Security headers on every response, host validation on every protected route, and a 32-character minimum bearer token.
- An 80 percent line coverage floor enforced by the gate.
