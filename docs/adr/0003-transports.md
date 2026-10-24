# 0003 Ship stdio and Streamable HTTP together

Date: 2026-10-24

## Context

Desktop clients launch a local MCP server over stdio. A shared deployment needs an HTTP endpoint that several clients reach. The 2026-07-28 revision removes sessions from the streaming transport, so a request carries everything it needs.

## Decision

stdio is the default and runs on a current-thread runtime. `--http` starts Streamable HTTP on axum with the SDK's stateless session manager, a bearer token gate compared in constant time, a 1 MiB body limit, host validation, and liveness, readiness, and metrics endpoints. Both transports are announced in the same build, and the server lists 2026-07-28 and 2025-11-25 as supported versions.

## Consequences

One binary serves every client shape. HTTP mode refuses a non-loopback bind without a token, so a careless public start fails instead of running open. The operator gets a health check that touches no upstream and a readiness check that does not gate on Serper. Logs go to stderr in both modes, which keeps the stdio channel clean.
