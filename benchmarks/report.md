# OwnSERP against garylab's Python server

Measured on 2026-10-24 on one Linux x86_64 machine, with Rust 1.99.1 and Python 3.13. Both servers were driven over stdio with plain JSON-RPC by `benchmarks/compare.py`, with the same `SERPER_API_KEY` and live Serper calls. The run spent 25 credits.

The Python server is `serper-mcp-server 0.0.10` from PyPI, the package the plan identified as the one in daily use. It does not start on a fresh install: its `mcp>=1.6.0` range resolves to mcp 2.3.0, which removed the `Server.list_tools` decorator it uses, and mcp 1.6.0 in turn fails against pydantic 2.12. The numbers below were taken with `mcp==1.6.0` and `pydantic==2.11.4` pinned.

## Headline

- Startup to a completed `initialize`: 6.9 ms for OwnSERP, 785.5 ms for the Python server.
- Resident memory after initialization: 12.7 MB for OwnSERP, 67.6 MB for the Python server.
- `tools/list` latency: 1.2 ms for OwnSERP, 7.5 ms for the Python server.
- Shared-tool payloads, 13 tools called once each with default settings: 63,850 bytes for OwnSERP, 132,684 bytes for the Python server. OwnSERP returns 51.9 percent fewer bytes.
- Ten live searches in sequence: OwnSERP p50 1,292 ms, p95 1,840 ms, mean 1,367 ms; Python p50 1,625 ms, p95 2,735 ms, mean 1,817 ms. Neither had an error.
- Tool catalog: OwnSERP 44,688 bytes for 17 tools; Python 14,412 bytes for 13 tools. OwnSERP pays about three times the discovery bytes to pin an input and output schema on every tool.

The load test is sequential, so the Serper round trip (typically one to two seconds) dominates the wall clock. The gap comes from what the server does around the call: OwnSERP keeps one pooled HTTPS client and one rate limiter for the process lifetime, while the Python server opens a new aiohttp session per request. Treat the percentages as indicative; individual queries vary by a second or more run to run.

## Per tool

Bytes are the whole JSON-RPC response. Credits are what Serper reported for that call.

- Search: OwnSERP 3,076 bytes, 1 credit. Python 4,016 bytes.
- Images: OwnSERP 2,672 bytes, 1 credit. Python 5,837 bytes.
- News: OwnSERP 4,410 bytes, 1 credit. Python 6,961 bytes.
- Places: OwnSERP 3,344 bytes, 1 credit. Python 4,937 bytes.
- Maps: OwnSERP 8,029 bytes, 3 credits. Python 30,500 bytes.
- Videos: OwnSERP 4,150 bytes, 1 credit. Python 7,198 bytes.
- Shopping: OwnSERP 15,837 bytes, 2 credits. Python 27,527 bytes.
- Scholar: OwnSERP 4,981 bytes, 1 credit. Python 7,629 bytes.
- Patents: OwnSERP 6,871 bytes, 1 credit. Python 40,719 bytes.
- Autocomplete: OwnSERP 692 bytes, 1 credit. Python 1,150 bytes.
- Lens: OwnSERP 2,117 bytes, 3 credits. Python 2,378 bytes.
- Scrape: OwnSERP 663 bytes, 2 credits. Python 378 bytes. OwnSERP returns Markdown by default; the Python server sends `includeMarkdown` off and returns plain text.
- Reviews: OwnSERP 7,207 bytes, 1 credit. Python fails. Its request model requires `fid`, so a call with `cid` or `placeId` alone is rejected, although Serper accepts any one of the three.

OwnSERP also exposes `serper_locations` (free), `serper_health` (free), `serper_session` (free), and `serper_search_batch` (three times the single price per query), none of which the Python server has.

## Coverage and parameters

OwnSERP covers every endpoint the Serper API publishes: search, images, news, places, maps, videos, shopping, scholar, patents, autocomplete, reviews, lens, scrape, locations, health, and the array batch form. The Python server covers twelve of the search and media endpoints plus scrape.

Parameter coverage against the contracts, where the Python server falls short:

- Maps takes `q`, `ll`, `placeId`, `cid`, `gl`, `hl`, `page`. It omits `num`, `tbs`, `autocorrect`, and `location`.
- Places and scholar omit `num` and `tbs`.
- Shopping omits `tbs`.
- Patents takes only `q`, `num`, and `page`.
- Scrape takes only `url` and `includeMarkdown`.
- Autocomplete accepts `location` and `page`, which Serper does not use for that endpoint.

OwnSERP validates every shared parameter, constrains `num` to 1 to 100, `page` to 1 and up, `tbs` to the five live values, and `gl` and `hl` to two-letter codes.

## Type and error handling

- The Python server requires `num` and `page` as strings. A caller that sends the integers the API documents gets a validation error, and the error is returned inside a normal content block with `isError: false`, so a client sees success and a text paragraph.
- OwnSERP takes the documented types, rejects an unknown argument before any request, and returns a structured error with a stable code, a message, and a remedy, with `isError: true`.

## Field parity

Default presets differ by design. OwnSERP `standard` keeps the fields an agent reasons with; the Python server emits a wider raw object. At `fields=full`, OwnSERP returns every field the Python server does for the shared endpoints, plus a pinned schema that names them.

The comparison found fields OwnSERP did not model at all, now fixed:

- Places and maps returned no `fid` or `placeId`, so a maps result could not be passed to reviews. Both are now in `standard`.
- Maps `phoneNumber` was unread.
- Shopping omitted rating, rating count, and product id.
- Scholar omitted pdf link, cluster id, and versions.
- Patents omitted inventor, language, publication date, and figures.
- Lens omitted the image URL.
- Images omitted thumbnail dimensions; videos omitted source.
- Patents results arrive in the `organic` array, and OwnSERP read a `patents` array, so the patents tool returned nothing. Fixed.
- Serper reports actual credits on every response, not only on scrape. OwnSERP records them now, and the cost object and ledger use the reported figure.

## Verdict

OwnSERP starts about a hundred times faster, uses about a fifth of the memory, returns about half the bytes, and completed the same live load about a quarter faster. It covers four endpoints the Python server lacks and the full parameter set on the rest. The Python server is smaller to discover because it pins no schemas and returns raw JSON, it cannot start today without two pinned dependencies, and it reports some failures as success.

## Reproduce

```sh
cargo build --release
SERPER_API_KEY=... python3 benchmarks/compare.py --json-out benchmarks/results.json
```

The harness needs the Python server installed with `mcp==1.6.0` and `pydantic==2.11.4`; point `--python-server` at its entry point. Every call is live and spends credits.
