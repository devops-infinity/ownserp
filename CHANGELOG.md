# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog 2.0.0](https://keepachangelog.com/en/2.0.0/), and the project uses [semantic versioning](https://semver.org/).

The minimum supported Rust version is 1.89. Raising it is recorded in this file and ships in a minor release.

## [Unreleased]

### Added

- `serper_resolve` turns the `google.com/goto` links a search returns into the real destination URLs at no cost, through Serper's `/resolve` endpoint. When Google has not released a link yet, the response carries `retryAfter` and the same links are sent again after that many seconds.
- Web and image results report `displayedLink` beside each `link`, the readable form Google shows for a `google.com/goto` redirect.

### Changed

- A web search that leaves `num` out asks for 10 results, the size of one Google page, so the count no longer depends on Serper's own default.
- Search tools take `resolve_links`, which sends `resolveLinks` to Serper and skips the top-three link resolution a search now does before answering.
- The catalog budget in the registry test rises from 40 KB to 48 KB, because the tool list gained `serper_resolve`.

## [2.1.0] - 2026-10-30

### Added

- `ownserp update` reports the latest release and the upgrade command for this install channel, and `--json` gives the same answer for scripts.
- `ownserp audit remove --yes` deletes the live ledger, its archives, and the arguments key, and prints each path it removed.
- `ownserp audit rotate --archive-damaged` archives a ledger whose chain is broken, up to the last row that verified, so a fresh chain can start.
- `ownserp audit verify` and `ownserp audit rotate` accept `--profile` and `--ledger-dir`, and every audit message prints the resolved path.
- `serper_session` reports a `ledger` block with `healthy`, `queued`, `lost`, and `creditsStale`, and `liveBalanceError` when the balance probe fails.
- `tools/smoke.sh` probes a deployed server, and the gate now checks suppression recheck dates and the manifest versions.

### Changed

- A paid tool call with no API key is refused before any network call with `serper.auth_missing`, and the cause names an unreadable keychain when that is the reason.
- An exhausted Serper balance reports `serper.credits_exhausted` and names the top-up step, instead of reporting a rate limit.
- The server's own deadline reports `serper.deadline_exceeded` instead of telling the client it cancelled.
- Every HTTP refusal carries the `code`, `message`, and `remedy` object instead of a bare sentence.
- A disabled batch tool reports `serper.tool_disabled` and names `enable_batch`.
- Ledger faults report `serper.ledger_fault`, and the remedy names the repair command.
- Paid tools no longer advertise the read-only hint, and only free tools report idempotent.
- `ownserp doctor` prints the cause and the fix when a setting cannot be resolved, and reports the credit ceilings, session budget, request timeout, bearer count, and bind.
- The CLI prints an error's remedy after its cause chain.
- `ownserp config set-api-key` says when `SERPER_API_KEY` will take precedence over the key it just stored.
- `ownserp config show` help points at `ownserp doctor` for the full settings report.
- `rmcp` is updated to 3.8.0.

### Fixed

- TLS verification no longer depends on the host trust store. The client trusts the Mozilla root set bundled into the binary and adds the platform roots when they load, so a container without `ca-certificates`, a keychain-less macOS shell, and a static build all verify Serper and the release lookup. Before this, a macOS shell whose keychain chain could not be built failed every handshake with `UnknownIssuer`, and a host with no readable CA bundle failed at client build with `No CA certificates were loaded from the system`.
- A missing or unreadable keychain no longer looks the same to the caller as a key that was never set.
- A truncated search result reports the delivered count and names the next step, and a trimmed batch says how many queries were left out.
- A paid response that fails shaping is cached before shaping, so a retry does not pay Serper again.
- The ledger's shutdown flush takes the file lock and notices a replaced file.
- A failed ledger append keeps its rows for a retry and clears the unhealthy flag on the next successful write.
- A final ledger row that only lost its newline is kept and terminated instead of dropped.
- A ledger row now carries the format version, and a file written by a newer binary is refused by name.
- The budget left in `serper_session` counts queued credits, so it matches what the guard will allow.
- A connection that stays silent for ten seconds is dropped, so an idle socket cannot hold a slot forever.
- The stdio transport cancels an in-flight call on a signal and has a call deadline.
- A Host that carries a trailing dot is refused by one rule instead of two different messages.
- An arguments key that another user can read is refused at startup.

## [2.0.0] - 2026-10-25

### Added

- `OWN_SERPER_PER_CALL_CREDIT_CEILING` and `OWN_SERPER_SESSION_BUDGET_CREDITS` set the two spending limits from the environment, so a container can carry them without a profile.
- `ownserp config show` accepts `--profile` and `--ledger-dir`.
- A knowledge graph, answer box, or people-also-ask object carries a `truncated` marker when one of its text fields was cut.

### Changed

- A call that Serper answers with a 5xx status or a redirect is now recorded at zero credits, because only successful calls are billed. A timeout or a cancel keeps the estimate.
- A lock wait that runs out of time no longer marks the ledger broken for the rest of the process. The call fails with a busy error and the next one tries again.
- An upstream 400 or 422 now reports `serper.bad_request`, and a JSON parse or shaping failure carries its own remedy instead of advice to check disk space.
- `LedgerRow`, `Reservation`, and `ToolTotals` are re-exported from `ownserp-core`.
- The HTTP request deadline follows `request_timeout_secs` and cancels the call so its row is written, and shutdown cancels in-flight work and waits briefly for the ledger.
- A failed connect is recorded at zero credits.
- A scrape keeps its line breaks, headings, lists, and code blocks.
- The result budget shrinks secondary sections before it drops paid results, keeps the largest prefix that fits, and keeps the paging token.
- `hl` accepts regional codes such as `zh-cn` and `pt-br`.
- A connection that sends no header within ten seconds is dropped, so idle sockets cannot hold every connection slot.
- A batch entry that carries an upstream message or status code now reports it instead of showing an empty success.
- Client-facing ledger errors name the file without its full path, and `ownserp doctor` reports a rejected key and exits 1.

### Fixed

- A scholar result's year is now read when Serper sends it as a number, and a review's author is read from `user.name`, which is where the live API puts it. Both fields used to come back empty.
- A cache entry larger than the byte cap no longer empties the cache, and a poisoned cache lock is recovered instead of failing every later call.
- The response cache eviction queue no longer grows without bound when entries are replaced or expire.
- A scrape whose response carries no credit figure is now booked with no actual credits and the 10-credit estimate, instead of recording 2 as if it were the real price.
- A 400 answer that says the balance is empty now reports `serper.rate_limited`, so an agent stops retrying against an exhausted account.
- The API key is trimmed when it is read, so a CRLF from a `.env` file no longer makes every request fail.
- `ownserp audit rotate` re-checks the ledger file after taking its lock, so a rotate that lost the race cannot archive the wrong file, and the rename is made durable.
- A paid call whose row cannot be written waits in a bounded queue and is written on the next append, instead of being lost.
- Dropping a reservation no longer waits on the ledger lock, and a call that is already in the chain no longer holds its share of the budget twice.
- `ownserp audit verify` reports a missing ledger, a damaged tail, and a gap in the sequence instead of passing them.
- The ledger open refuses a symlinked file, and a file that holds something other than a ledger is refused instead of cut to zero bytes.
- Ledger reads and budget reservations no longer run on the async thread.
- The profile version is checked before its keys, an unresolvable default directory is an error, and the keychain error names its cause.
- The bearer scheme is case-insensitive, Host entries compare ports and case, an Origin on a protected route is refused, and a token that can never be sent is refused at startup.
- The release tooling writes the new changelog heading in the right place, runs the strict gate before a tag, fills a draft release before publishing it, syntax-checks every shell file, and passes the tag through the environment.

### Removed

- The repository no longer ships a project `.mcp.json`, so Claude Code does not pick up a server configuration from a checkout of this repository.
- **Breaking:** removed the client-side QPS limit and the HTTP inbound rate limit, so requests to Serper are no longer throttled locally.
- **Breaking:** removed the `--qps` flag and the `qps` and `http_qps` profile keys and environment variables. A profile that still carries either key now fails to load.
- Removed the `QpsLimit`, `PackTier::qps()`, and `CancelledBeforeSend` public items from `ownserp-core`.

## [1.3.1] - 2026-10-25

### Fixed

- A call that was paid for and written to the ledger no longer comes back as an error. The cost object reads the totals this process already holds, so a busy ledger can no longer fail a result whose row was written.
- Ledger reads and budget reservations no longer run on the async thread, so a held ledger lock cannot stall the server's other requests while it waits.
- A call that Serper rejects with a 4xx status, or answers with a rate limit, is now recorded at zero credits instead of the estimate, because nothing was billed.

## [1.3.0] - 2026-10-25

### Changed

- Any number of `ownserp serve` processes on one machine now share one ledger, one hash chain, and one running total. A process holds the file lock only while it reads the rows written since its last call and appends its own row, instead of holding it for as long as it runs.
- `serper_session` reports the credits written by every process, and `session_budget_credits` counts all of them. A credit that another process has reserved but not yet written is invisible to that check, so the budget can be passed by at most the running process count times the per-call credit ceiling.
- `ownserp audit rotate` no longer refuses while a server runs. A running server notices the new file and starts a fresh chain on its next write.
- `args.key` is created by linking a temporary file into place, so several processes starting at once read one key instead of the last one to rename its own.
- **Breaking:** five public items in `ownserp-core` changed name: `usd_for` is now `usd_for_call`, `PackTier::usd_per_thousand` is now `usd_per_1k`, `MapViewport::as_query` is now `as_str`, `NAME` is now `APP_NAME`, and `SerperClient::has_key` is now `has_api_key`. Code that calls those items needs the new names. The ledger file format and the chain hash did not change, so an existing `calls.jsonl` still verifies and keeps growing.

## [1.2.1] - 2026-10-25

### Changed

- The MCPB bundle pins entry timestamps and drops extra zip attributes, so the same inputs produce the same file hash on every build.
- `server.json` lists the npm package, and `tools/publish-npm.sh` sets `mcpName` before packing, so the MCP Registry can check ownership of the published package.
- `RELEASING.md` gives the publish step for every channel: npm, crates.io, the MCP Registry, Homebrew, and the container, with the order each one needs.

### Fixed

- The MCP Registry entry stopped matching its bundle after a release rerun, because the rebuilt zip carried new timestamps and the recorded hash went stale.

## [1.2.0] - 2026-10-24

### Changed

- Release packaging moved to cargo-dist: prebuilt archives for macOS, Linux, and Windows, shell and PowerShell installers, a Homebrew formula that installs the binary, and an npm package that selects the platform archive.
- Releases are built by `.github/workflows/release.yml` on a macOS runner when a `v*` tag is pushed; the workflow attaches every archive, the installers, the formula, the npm package, and the MCPB bundle to the release.
- The Docker example sets `OWN_SERPER_ALLOWED_HOSTS`, because the image binds `0.0.0.0` and the derived allowlist matches no client Host.

### Fixed

- The tag publish hint no longer references a removed archive variable, so `--tag` exits cleanly after creating the tag.

## [1.1.0] - 2026-10-24

### Changed

- Release tooling: the npm loader stamp comes from the `x86_64-unknown-linux-gnu` archive, the line is verified after writing, and `--tag` refuses unless the pinned digest is committed.
- `tools/verify.sh` names every skipped check, and `OWN_SERPER_STRICT_GATE=1` turns a skip into a failure; the release runs that way.
- `cache_max_bytes` is capped at 1 GiB, `request_timeout_secs` must be 1 to 600, and `ledger_dir` must be an absolute path; each is refused at load with a message that names the field.
- The container copies only `.cargo/config.toml`, ignores `.cargo` registry credentials, and reads the healthcheck port from `OWN_SERPER_BIND`.
- The Codex setup example uses `env_vars`, so the API key never enters a command line.

### Fixed

- `serper_reviews` and `serper_lens` reserved the call price twice, so a one-credit call could not fit a one-credit session budget. Each call reserves once now.
- A failed or cancelled call was booked at zero credits even when the request may have reached Serper. It now charges the estimate, so a timeout cannot spend outside the session budget.
- A profile that fails a stat fell back to defaults that carry no spend cap. It is now an error, the file must be regular, and the read is capped at 64 KiB.
- The ledger checks the file type before reading, compares the opened descriptor with the path, and sets permissions through the descriptor, so a symlink or directory swap is refused.
- The ledger lock error names `OWN_SERPER_LEDGER_DIR` as the way to run a second client.
- The HTTP rate limiter runs before the bearer gate, connections are capped at 256, the Host check accepts the HTTP/2 authority, and shutdown joins the server task after the grace period.
- `SerperClient::with_base_url` requires https outside loopback and sends the API key only to a Serper host or a loopback test server.

## [1.0.0] - 2026-10-24

### Added

- `ownserp completions <shell>` prints completions for bash, zsh, fish, elvish, and PowerShell, and `ownserp man` prints the manual page.
- Release archives carry `ownserp.bash`, `_ownserp`, `ownserp.fish`, and `ownserp.1`.
- A network failure carries up to three sanitized causes in the tool error, so a DNS, TLS, or connection failure is visible to the caller.
- The profile carries a `version` key; a newer schema version is refused by name.

### Changed

- `ownserp doctor` exits 1 when no key is configured or when the Serper health probe fails.
- The HTTP transport gives a request body 30 seconds to arrive, so a stalled upload cannot hold a concurrency slot for the full request deadline.
- The ledger releases its lock before fsync, so a slow disk no longer blocks the totals and budget readers.
- `tools/release.sh` splits package and tag: packaging stamps the archive digest into the npm loader, builds every target, and tagging refuses unless that stamp is committed.
- The container image carries a health check against `/healthz`.

## [0.1.5] - 2026-10-24

### Fixed

- On Windows, the profile and ledger use `%APPDATA%\ownserp` and `%LOCALAPPDATA%\ownserp` when `HOME` is unset, instead of the working directory.

## [0.1.4] - 2026-10-24

### Added

- `tools/registry-auth.sh`, which refreshes the GitHub, npm, crates.io, MCP Registry, and ghcr sessions from the environment variables in the shell.
- `serve --no-http` and `serve --no-batch`, so a profile setting can be turned off from the command line.

### Changed

- The container image is public, so an anonymous pull works.
- The MCP Registry entry for 0.1.3 lists the npm package, and the README carries the `mcp-name` marker so a later crate release can list the cargo package too.
- The HTTP request deadline is 120 seconds, down from 300.
- The HTTP transport rejects any request that carries an `Origin` header and answers a missing or bad bearer token with `WWW-Authenticate: Bearer`.
- A loopback bind accepts `localhost`, `127.0.0.1`, and `::1` as the Host, and `OWN_SERPER_ALLOWED_HOSTS` is resolved with the rest of the settings instead of read at serve time.
- The archive and the container image are built with the `dist` profile.
- `tools/release.sh --prepare` stamps the `ownserp-core` requirement, the README install version, the npm package and loader, and the Dockerfile label, and it verifies `Cargo.lock` and every other stamp before it reports success.
- A 5xx response is still retried within the attempt cap, on the rule that a failed call is not billed; the 0.1.3 note that described connect-only retries is superseded.
- The container image installs the CA bundle, so outbound TLS uses the system trust store.
- A retry backoff ends as soon as the client cancels.
- The USD rate must be finite and non-negative.
- `--version` answers under every subcommand.

### Fixed

- A ledger whose last line is valid JSON but has no newline no longer breaks the hash chain on the next start.
- A failed ledger append stops further paid calls until the ledger is repaired.
- A failed upstream call records zero credits instead of its estimate.
- A batch response that is not an array is an error instead of an empty success.
- `serper_locations` applies the result budget.
- The cache charges per-entry overhead against its byte cap and stores the response without copying it.
- An oversized `queries` array is refused before it is parsed.
- A missing explicit `--profile` is an error instead of silently dropping the budget.
- `config set-api-key` caps its stdin read and refuses a terminal, so a typed key is never echoed.
- `audit verify --file` reports a missing ledger instead of verifying zero rows.
- A URL is normalized before it is sent upstream.
- Logs on a pipe or in a file carry no ANSI escapes.
- The `.sha256` file names its archive, so `sha256sum -c` verifies it.
- The npm package ships both license texts.

### Security

- `args.key` must be a regular file, and it is written through a temporary file before it is renamed into place.
- The HTTP transport enforces Origin validation and serves legacy protocol clients statelessly instead of failing the handshake.

## [0.1.3] - 2026-10-24

### Added

- `serper_health`, a free tool that reports the status of the Serper API.

### Changed

- The npm loader pins the archive SHA-256 inside the published package, accepts redirects only to GitHub release hosts, downloads with a deadline, and extracts into a temporary directory before moving the binary into place.
- The batch tool takes one rate-limit permit per query, and a query whose response cannot be read now carries an error marker instead of an empty result.
- A retry covers a failed connect only. A timeout can follow a request the upstream already billed, so it is reported instead of repeated.
- The upstream client refuses a redirect, so the API key cannot follow one to another host.

### Fixed

- A cache hit no longer adds the reported credits to the session totals.
- An upstream body is cached only after it projects cleanly, so a malformed body is not replayed for the life of the entry.
- The cache subtracts the bytes of a replaced entry, so its byte cap stays accurate.
- Health and balance bodies read through the 4 MiB cap.
- The result budget shrinks nested strings and arrays and keeps the essential fields when a result still passes the cap.
- Patents results read the `organic` array that Serper returns.
- Actual credits from every response are recorded in the cost object and the ledger.
- Places and maps return `fid` and `placeId`, so a review lookup can follow a result, and maps reads `phoneNumber`.
- Shopping returns rating and product id, scholar returns the pdf link and cluster id, patents returns inventor, language, publication date, and figures, lens returns the image, and images and videos return the missing media fields.
- The public documentation carries no personal branch prefix or npm username.
- The ledger reads a line with a byte cap, refuses to write an oversized row, syncs each row to disk, sets owner-only permissions when the directory and file are created, and logs an append failure instead of failing a call that already spent credits.
- An unreadable or wrong-length `args.key` stops the server with a message instead of being replaced silently.
- The HTTP server holds at most 128 requests at once, gives each one a 300 second deadline, rate-limits before the bearer check, and compares tokens by digest.
- The commit hook fails when the secret scanner is missing, and both gates use the same `.gitleaks.toml`.
- A URL query in an error message is redacted before it reaches a log or a tool error.
- The sanitizer also strips line and paragraph separators, the variation selector supplement, and the Unicode fillers, and a cleaned key collision keeps the first value.
- The one-line text summary caps its title and page token, so it cannot pass the result budget.
- The release script verifies that every version stamp landed and cleans its temporary files.
- The npm token setup checks the new token before replacing the working one.
- `.dockerignore` excludes environment and key files.
- The reinstall script swaps the binary by rename, so an interrupted install leaves the previous copy in place.

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

[Unreleased]: https://github.com/devops-infinity/ownserp/compare/v1.3.1...HEAD
[1.3.1]: https://github.com/devops-infinity/ownserp/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/devops-infinity/ownserp/compare/v1.2.1...v1.3.0
[1.2.1]: https://github.com/devops-infinity/ownserp/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/devops-infinity/ownserp/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/devops-infinity/ownserp/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/devops-infinity/ownserp/compare/v0.1.5...v1.0.0
[0.1.5]: https://github.com/devops-infinity/ownserp/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/devops-infinity/ownserp/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/devops-infinity/ownserp/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/devops-infinity/ownserp/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/devops-infinity/ownserp/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/devops-infinity/ownserp/releases/tag/v0.1.0

[2.0.0]: https://github.com/devops-infinity/ownserp/compare/v1.3.1...v2.0.0

[2.1.0]: https://github.com/devops-infinity/ownserp/compare/v2.0.0...v2.1.0
