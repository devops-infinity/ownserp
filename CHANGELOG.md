# Changelog

All notable changes to this project are recorded here. The format follows Keep a Changelog 2.0.0, and the project uses semantic versioning.

## Policy

The minimum supported Rust version is 1.89. Raising it is recorded in this file and ships in a minor release.

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
