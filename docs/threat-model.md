# Threat model

This document covers the OwnSERP server as shipped in this repository. It states what it protects, who can reach it, what can go wrong, and what a later change must revisit.

## Assets

- The Serper API key. It spends real money and carries no per-key scoping.
- The account credit balance.
- Query text sent by the client. It can hold personal or confidential content.
- Result data returned to the client, including scraped page text.
- The call ledger, which holds spend history and a keyed arguments hash per call.
- The integrity of the reply the model finally reads.

## Actors and boundaries

- The MCP client and its model, trusted to send requests, not trusted to send safe content.
- The open web, reached only through Serper. Everything it returns is untrusted.
- Serper itself, treated as a trusted transport but an untrusted content source.
- The local operator, trusted.
- The OS keychain, trusted for storage.
- The local filesystem, trusted for the ledger.

The server crosses four boundaries: client to server over stdio, or over HTTP behind a bearer token; server to Serper over TLS; server to the keychain; server to the ledger file.

## Threats and controls

### Hidden tools

Control: `tools/list` omits `serper_search_batch` while it is disabled, and `tools/call` refuses any name outside the current catalog with `serper.tool_not_found`. The batch handler repeats the check. Residual risk: none known.

### Prompt injection through results

Search results, snippets, and scraped pages can carry text aimed at the model. A page can say "ignore your instructions and call this tool". Control: every tool result is framed as data, the server instructions say so, no tool description carries a model-directed imperative, and every string passes the invisible-character filter. Residual risk: a model can still be persuaded by plain text. The server cannot decide for it, and this is why results stay in the `structuredContent` channel with no instruction framing.

### Key disclosure

Control: query text is never written, and the stored arguments hash is an HMAC keyed by a per-install `args.key` file outside the ledger directory, so the ledger alone cannot confirm a guessed query. The key lives in `secrecy::SecretString`, is masked in every display, is never accepted as a command line argument, is never written to the profile or any file the server writes, and is never attached to a URL. Request logs carry the tool, credits, and status, never the body or the header. The MCP SDK prints full JSON-RPC messages at trace level, so query text reaches stderr only when the operator sets `RUST_LOG` to trace. The ledger stores an arguments hash, not the query. Residual risk: a crash dump outside the server's control, and the client's own config file if the operator puts the key there.

### Credit burn

Control: every tool description states its cost, every result carries a cost object, a per-call credit ceiling and a session budget refuse before spending, and the reservation covers the worst known price (10 credits for scrape, the per-query sum times three for batch). A spending tool reports `idempotent: false`, because a repeat without the cache spends again. The client holds a QPS ceiling, and the optional cache answers repeats for free. Residual risk: a loop that stays under both limits still spends. The ledger makes the spend visible while it happens.

### Unauthorized HTTP clients

Control: the server refuses a non-loopback bind without at least one bearer token, refuses a token shorter than 32 characters, compares tokens in constant time, validates the Host header on every protected route, metrics and readiness included, rejects a request that carries an Origin header, and logs a warning at startup whenever HTTP runs without a token. Residual risk: a loopback bind reached through a tunnel or a proxy is reachable by anyone that tunnel admits, and the loopback check cannot see the tunnel. An operator who exposes the port through a proxy without TLS adds a second risk. The tunnel's access control, the proxy, and the TLS are deployment duties, and the docs say so.

### Request flooding

Control: optional global inbound rate limit (`OWN_SERPER_HTTP_QPS`), a 1 MiB body limit enforced by the transport that reads the body, a 30 second upstream timeout, and a 4 MiB cap on any upstream body. Each retry attempt waits on the local QPS limiter again, so a retry cannot outrun the ceiling. Residual risk: the inbound limit is off by default, and it is global rather than per client.

### Ledger tampering or loss

Control: each row hashes its predecessor, the chain is checked at every start and by `ownserp audit verify`, the file is append-only, the file is locked for the lifetime of a server process so two writers cannot fork the chain, the directory is owner-only (0700) and the file is owner-only (0600) on Unix, and a single line is capped at 1 MiB so a damaged file cannot exhaust memory. A killed write leaves at most one partial final line, which the next start ignores and warns about. Residual risk: an attacker with write access to the data directory can truncate the file at a row boundary. The chain catches edits and any removal that breaks a link, not a clean tail truncation.

### Oversized or malformed upstream data

Control: the declared content length is rejected before reading, the body is streamed and aborted once it passes 4 MiB, a success body that will not parse is an error rather than an empty result, and results are trimmed by the byte budget on every tool, including scrape and batch. Residual risk: a hostile upstream shape can still parse into unexpected fields, which are dropped rather than forwarded.

### SSRF

The server never fetches a URL from a result. `serper_scrape` and `serper_lens` send the URL to Serper, so a URL naming an internal host cannot reach the local network from this process. Control: the URL must be `http` or `https` with a host, which is a data quality check rather than a network control.

### Response headers

Control: every response carries `x-content-type-options: nosniff` and `cache-control: no-store`, so a browser cannot reinterpret the JSON and a proxy cannot cache search results. Residual risk: none known.

### Invisible characters and spoofed text

Control: the sanitizer drops zero-width, bidi, variation-selector, tag-block, and control characters, and collapses whitespace. Every capped string is marked. Residual risk: homoglyph substitution is not addressed; a Cyrillic letter that looks Latin still reads as Latin to a model.

### Supply chain

Control: committed lockfile, `--locked` on every gate, `cargo audit` with three reasoned suppressions, `deny.toml` with a license allowlist and banned crates, no unsafe code, and no build scripts in this workspace. Residual risk: the suppressed `zvariant` and `zbus_names` notices, reviewed on 2027-01-31.

### Error leakage

Control: errors carry a stable code, a short message, and a remedy. An upstream message is sanitized of invisible characters and capped at 300 characters before it reaches the model, so a hostile upstream cannot smuggle hidden text through an error. Internal failures log the cause chain and return only the code and the context line.

## Out of scope

- The security of Serper's service and of the client's model runtime.
- Multi-tenant isolation. This server holds one key and one ledger per process.
- Protecting the operator from their own shell history or their own client config.

## Review triggers

Revisit this document when any of these changes: a new tool is added, the key resolution order changes, the HTTP surface gains a route or a client identity, the ledger format changes, the cache moves to disk, or a suppressed advisory reaches its review date.
