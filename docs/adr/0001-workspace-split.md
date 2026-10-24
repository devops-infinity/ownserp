# 0001 Split the crate into ownserp and ownserp-core

Date: 2026-10-24

## Context

The server has two jobs: speak the MCP protocol, and talk to the Serper API while accounting for credits. The binary needs a process entry point, argument parsing, and signal handling. The engine needs to be reachable from tests without starting a process.

## Decision

Two crates in one Cargo workspace. `ownserp-core` holds the client, tool registry, shaping, ledger, cache, settings, and both servers. `ownserp` holds the CLI and does nothing else. Dependencies point one way: the binary depends on the library, never the reverse.

## Consequences

Integration tests drive the real engine through the public API, so the facade is proven by the test suite. Compile times stay workable because the engine and the CLI build in parallel. A future embedding of the engine in another program needs no new crate.
