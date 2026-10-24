# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/), and the project uses [semantic versioning](https://semver.org/).

The minimum supported Rust version is 1.89. Raising it is recorded in this file and ships in a minor release.

## [Unreleased]

### Added

- `serper_health`, a free tool that reports the status of the Serper API.
- A benchmark harness and a comparison report against the Python Serper MCP server.

### Fixed

- Patents results read the `organic` array that Serper returns.
- Actual credits from every response are recorded in the cost object and the ledger.
- Places and maps return `fid` and `placeId`, so a review lookup can follow a result, and maps reads `phoneNumber`.
- Shopping returns rating and product id, scholar returns the pdf link and cluster id, patents returns inventor, language, publication date, and figures, lens returns the image, and images and videos return the missing media fields.
- The public documentation carries no personal branch prefix or npm username.

## [0.1.2] - 2026-10-24

### Fixed

- The `ownserp` crate description starts with OwnSERP, so crates.io shows the product name.
- The secret scan reads source files only and skips build output, so the gate cannot fail on a generated file.

### Added

- An npm loader, `@devops-infinity/ownserp`, that downloads the pinned Linux x86_64 binary, checks it against the published SHA-256, and runs it.
- A project `.mcp.json` that starts the server with the key from the client environment.
- A Linux MCPB bundle on the release page.

### Changed

- `server.json` lists the cargo and npm packages, so the MCP Registry publishes both.

## [0.1.1] - 2026-10-24

### Changed

- The annotated release tag and the release command hint carry the OwnSERP label.

## [0.1.0] - 2026-10-24

### Added

- MCP server over stdio and Streamable HTTP, speaking 2026-07-28 and 2025-11-25.
- Fifteen default tools covering search, images, news, places, maps, videos, shopping, scholar, patents, autocomplete, reviews, lens, scrape, locations, and session statistics.
- `serper_search_batch`, behind the `enable_batch` setting, with up to 100 queries per call at three times the single price.
- Field presets `minimal`, `standard`, and `full`, with compact serialization and a 256 KiB result budget.
- A per-call cost object, a hash-chained ledger, and `ownserp audit verify`.
- A per-call credit ceiling, a session budget, a client-side QPS limit, and retry with backoff for transient failures.
- A response cache with a TTL and a byte cap, off by default.
- Structured errors with stable `serper.*` codes and a remedy line.
- A binary CLI with `serve`, `doctor`, `config`, and `audit` commands.
- An exclusive ledger lock, owner-only ledger permissions, a per-line size cap, and a keyed arguments hash.
- Refusal of an unknown or disabled tool call, and reservation of the worst known credit cost before a call.
- Retries that pass through the local rate limiter, and upstream error text that is sanitized and capped.
- Security headers on every response, host validation on every protected route, and a 32-character minimum bearer token.
- The OwnSERP label in the CLI name and version line, the MCP server identity, the README, the release title, the bundle display name, the image labels, and both crate descriptions.
- The `search-api` keyword, which replaces `model-context-protocol` because crates.io rejects a keyword above 20 characters.
- The workspace README in both packages, so crates.io renders it.
- A registry manifest, a bundle manifest, and a Dockerfile.
