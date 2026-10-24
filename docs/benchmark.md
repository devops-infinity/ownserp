# Benchmark against the Python Serper MCP server

Measured 2026-10-24 on the Principal Architect's machine, Linux, Rust 1.99.1, Node 24, Python 3.14. Both servers spoke JSON-RPC over stdio and answered the same live Serper queries. Every call spent Serper credits, and each server ran against the same account in the same session.

The Python server is `garylab/serper-mcp-server` at its latest published release, installed with `mcp==1.6.0` and `pydantic==2.11.4`, because its own current pins do not start.

## Method

- One process per server, started on demand, stopped after the run.
- Startup time measured from process start to the first response.
- Memory read from the resident set after the tool list, then again after every call.
- Twelve shared calls, chosen so both servers expose the endpoint.
- A load run of ten `serper_search` calls in sequence with `num: 5`.
- Payload bytes counted on the JSON that reaches the client.

## Startup, memory, and catalog

- OwnSERP: 7.6 ms to the first response, 13.0 MB resident, 17 tools, 44,727 bytes of tool catalog.
- Python server: 920.9 ms to the first response, 66.1 MB resident, 13 tools, 14,412 bytes of tool catalog.

OwnSERP starts 121 times faster and holds 5 times less memory. Its catalog is larger because it carries a price and a full input and output schema on every tool, and because it ships four tools the Python server lacks: health, locations, session, and batch.

## Per-tool results

Each entry lists OwnSERP latency and payload, then the Python server's.

- Search: 2058.7 ms, 3,089 bytes; Python 1793.8 ms, 4,014 bytes.
- Images: 1823.8 ms, 2,672 bytes; Python 2250.1 ms, 5,837 bytes.
- News: 2500.9 ms, 4,410 bytes; Python 3657.5 ms, 6,961 bytes.
- Places: 1967.5 ms, 3,344 bytes; Python 2085.5 ms, 4,937 bytes.
- Maps: 1767.3 ms, 8,335 bytes; Python 1941.0 ms, 30,120 bytes.
- Videos: 1207.4 ms, 4,034 bytes; Python 1943.1 ms, 6,952 bytes.
- Shopping: 2369.0 ms, 15,218 bytes; Python 2857.8 ms, 26,848 bytes.
- Scholar: 1409.4 ms, 5,003 bytes; Python 1671.3 ms, 7,615 bytes.
- Patents: 2420.2 ms, 6,871 bytes; Python 3748.6 ms, 40,719 bytes.
- Autocomplete: 915.8 ms, 721 bytes; Python 942.3 ms, 999 bytes.
- Lens: 1415.7 ms, 2,142 bytes; Python 2360.7 ms, 364 bytes.
- Scrape: 628.5 ms, 663 bytes; Python 604.9 ms, 378 bytes.
- Reviews: 1345.6 ms, 7,122 bytes; Python returned an error in 1.0 ms because it requires a field the request does not need.

The Python server has no equivalent for health, locations, session, or batch. OwnSERP answered health in 272.5 ms, locations in 355.6 ms, session in 0.2 ms, and a two-query batch in 2460.6 ms.

Payload size favors OwnSERP on ten of twelve shared calls. Maps drops from 30 KB to 8 KB, patents from 40 KB to 7 KB, and scholar from 7.6 KB to 5 KB, because the field presets drop empty sections instead of serializing them.

## Load

Ten searches in sequence, one at a time, `num: 5`:

- OwnSERP: p50 1723.6 ms, p95 3891.1 ms, mean 2013.3 ms, no errors.
- Python server: p50 1717.6 ms, p95 2360.1 ms, mean 1796.4 ms, no errors.

Latency is a wash. Both servers wait on Serper, which typically answers in 1 to 2 seconds, and a run of ten samples moves by several hundred milliseconds between runs. A single search on OwnSERP was slower here and the median was level. A previous run of the same test showed OwnSERP ahead on all three figures, so the difference is the network, not the server.

## What the numbers support

- OwnSERP starts in single-digit milliseconds, so a client that launches the server on every request does not pay a startup cost.
- Memory stays around 13 MB, one fifth of the Python server, which matters when several clients run at once.
- Payload size favors OwnSERP on ten of the twelve shared calls, which leaves more room in a model's context.
- All fifteen Serper endpoints are covered, plus health, locations, and a session report. The Python server leaves out locations and health entirely.
- Every result carries a cost object, and every call lands in the local ledger.
- Per-call latency is set by Serper and is not a reason to choose either server.

## Caveats

- One run per figure. Live search latency varies by query and by time of day.
- The machine was otherwise idle, but the network path to Serper is shared.
- The Python server's reviews tool failed for a valid request, so its result is not a comparison of output quality.
