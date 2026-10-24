# 0002 One typed error enum with stable codes

Date: 2026-10-24

## Context

An MCP client needs to tell a user what went wrong and what to do next. Serper returns `{message, statusCode, error}`, publishes no rate limit headers, and bills only successful calls. A bare string error loses the difference between a bad key, an exhausted balance, and a network blip.

## Decision

One `Error` enum in `ownserp-core`, marked `#[non_exhaustive]`, built with `thiserror`. Each variant carries a stable code (`serper.auth_missing`, `serper.auth_failed`, `serper.rate_limited`, `serper.bad_request`, `serper.upstream_failed`, `serper.budget_refused`, `serper.cancelled`, and the internal family) and a remedy line. A tool failure returns `isError: true` with that JSON body, never a bare string. Dependency error types are converted at the boundary and never appear in a public variant.

## Consequences

A client can branch on the code without reading prose. Adding a variant cannot break a downstream match. The display text and the source accessor never both carry the same detail, so the cause chain stays walkable without repeating text.
