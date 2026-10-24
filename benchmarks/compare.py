#!/usr/bin/env python3
"""Benchmark OwnSERP against garylab's Python Serper MCP server.

Both servers are spoken to over stdio with plain JSON-RPC, so nothing here
depends on either project's SDK. Every call in the correctness pass is a live
Serper request and spends credits. Run it with SERPER_API_KEY set.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import statistics
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class CallResult:
    label: str
    latency_ms: float
    bytes: int
    is_error: bool
    payload: dict = field(default_factory=dict)


class McpServer:
    def __init__(self, name: str, command: list[str], protocol: str) -> None:
        self.name = name
        self.command = command
        self.protocol = protocol
        self.process: subprocess.Popen[str] | None = None
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.next_id = 0
        self.tools: list[dict] = []
        self.startup_ms = 0.0
        self.tools_ms = 0.0
        self.tools_bytes = 0
        self.idle_rss_kb = 0
        self.cpu_ms = 0.0

    def _reader(self) -> None:
        assert self.process is not None and self.process.stdout is not None
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def start(self) -> None:
        started = time.perf_counter()
        self.process = subprocess.Popen(
            self.command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
            env=os.environ.copy(),
        )
        threading.Thread(target=self._reader, daemon=True).start()
        self._request(
            "initialize",
            {
                "protocolVersion": self.protocol,
                "capabilities": {},
                "clientInfo": {"name": "ownserp-bench", "version": "0"},
            },
        )
        self.startup_ms = (time.perf_counter() - started) * 1000
        self._notify("notifications/initialized")
        started = time.perf_counter()
        message = self._request("tools/list", {})
        self.tools_ms = (time.perf_counter() - started) * 1000
        result = message.get("result", {})
        self.tools = result.get("tools", [])
        self.tools_bytes = len(json.dumps(result))
        self.idle_rss_kb = self._rss_kb()
        self.cpu_ms = self._cpu_ms()

    def _rss_kb(self) -> int:
        assert self.process is not None
        try:
            status = Path(f"/proc/{self.process.pid}/status").read_text()
            for line in status.splitlines():
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
        except OSError:
            pass
        return 0

    def _cpu_ms(self) -> float:
        assert self.process is not None
        try:
            stat = Path(f"/proc/{self.process.pid}/stat").read_text().split()
            ticks = int(stat[13]) + int(stat[14])
            return ticks * 10.0
        except (OSError, IndexError, ValueError):
            return 0.0

    def _send(self, message: dict) -> None:
        assert self.process is not None and self.process.stdin is not None
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def _notify(self, method: str) -> None:
        self._send({"jsonrpc": "2.0", "method": method})

    def _request(self, method: str, params: dict) -> dict:
        self.next_id += 1
        request_id = self.next_id
        self._send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + 120
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"{self.name}: {method} timed out")
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError(f"{self.name}: {method} timed out") from error
            if line is None:
                raise RuntimeError(f"{self.name}: process closed during {method}")
            line = line.strip()
            if not line:
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if message.get("id") == request_id:
                return message

    def call(self, label: str, tool: str, arguments: dict) -> CallResult:
        started = time.perf_counter()
        message = self._request("tools/call", {"name": tool, "arguments": self.adjust(arguments)})
        latency = (time.perf_counter() - started) * 1000
        if "error" in message:
            return CallResult(
                label=label,
                latency_ms=latency,
                bytes=len(json.dumps(message)),
                is_error=True,
                payload={"content": [{"type": "text", "text": json.dumps(message["error"])}]},
            )
        result = message.get("result", {})
        text = "".join(
            block.get("text", "") for block in result.get("content", []) if block.get("type") == "text"
        )
        return CallResult(
            label=label,
            latency_ms=latency,
            bytes=len(json.dumps(message)),
            is_error=bool(result.get("isError")) or text.lstrip().startswith("Error:"),
            payload=result,
        )

    def adjust(self, arguments: dict) -> dict:
        if self.name == "ownserp":
            return arguments
        adjusted = dict(arguments)
        for key in ("num", "page"):
            if key in adjusted and isinstance(adjusted[key], int):
                adjusted[key] = str(adjusted[key])
        return adjusted

    def stop(self) -> None:
        if self.process is None:
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()


def structured(result: CallResult) -> dict:
    value = result.payload.get("structuredContent")
    return value if isinstance(value, dict) else {}


def text_of(result: CallResult) -> str:
    blocks = result.payload.get("content") or []
    return "".join(block.get("text", "") for block in blocks if block.get("type") == "text")


def summarize_ours(result: CallResult) -> dict:
    body = structured(result)
    rows = body.get("results") if isinstance(body.get("results"), list) else []
    first = rows[0] if rows else {}
    return {
        "structured": bool(body),
        "results": len(rows),
        "first_keys": sorted(first.keys()) if isinstance(first, dict) else [],
        "sections": [k for k in ("knowledgeGraph", "answerBox", "peopleAlsoAsk", "relatedSearches") if k in body],
        "credits": (body.get("cost") or {}).get("credits"),
        "truncated": body.get("truncated"),
        "text": text_of(result)[:120],
    }


def summarize_python(result: CallResult) -> dict:
    text = text_of(result)
    parsed: object
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        parsed = None
    rows: list = []
    keys: list[str] = []
    if isinstance(parsed, list):
        rows = parsed
        keys = sorted(parsed[0].keys()) if parsed and isinstance(parsed[0], dict) else []
    elif isinstance(parsed, dict):
        for value in parsed.values():
            if isinstance(value, list) and value and isinstance(value[0], dict):
                rows = value
                keys = sorted(value[0].keys())
                break
    return {
        "structured": False,
        "results": len(rows),
        "first_keys": keys,
        "sections": [],
        "credits": None,
        "truncated": None,
        "text": text[:120],
        "pretty": "\n  " in text,
    }


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, int(round(fraction * (len(ordered) - 1))))
    return ordered[index]


def _python_reviews(server: McpServer, arguments: dict) -> dict:
    result = server.call("reviews", "google_search_reviews", arguments)
    return {
        "latency_ms": round(result.latency_ms, 1),
        "bytes": result.bytes,
        "is_error": result.is_error,
        **summarize_python(result),
    }


def load_test(server: McpServer, tool: str, query: str, count: int) -> dict:
    latencies: list[float] = []
    errors = 0
    wall_start = time.perf_counter()
    for index in range(count):
        result = server.call(f"load-{index}", tool, {"q": f"{query} {index}", "num": 5})
        latencies.append(result.latency_ms)
        if result.is_error:
            errors += 1
    wall = (time.perf_counter() - wall_start) * 1000
    return {
        "calls": count,
        "wall_ms": round(wall, 1),
        "p50_ms": round(percentile(latencies, 0.5), 1),
        "p95_ms": round(percentile(latencies, 0.95), 1),
        "mean_ms": round(statistics.fmean(latencies), 1) if latencies else 0.0,
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ours", default="target/release/ownserp serve --enable-batch")
    parser.add_argument("--python-server", default="/tmp/garylab-bench/.venv/bin/serper-mcp-server")
    parser.add_argument("--load", type=int, default=10)
    parser.add_argument("--skip-python", action="store_true")
    parser.add_argument("--json-out", default="benchmarks/results.json")
    args = parser.parse_args()

    if not os.environ.get("SERPER_API_KEY"):
        print("SERPER_API_KEY is not set", file=sys.stderr)
        return 2

    ours = McpServer("ownserp", args.ours.split(), "2026-07-28")
    python = None if args.skip_python else McpServer("serper-mcp-server", [args.python_server], "2025-06-18")

    ours.start()
    if python:
        python.start()

    ours_names = {tool["name"] for tool in ours.tools}
    python_names = {tool["name"] for tool in python.tools} if python else set()

    cases = [
        ("search", "serper_search", "google_search", {"q": "apple inc", "num": 10}),
        ("images", "serper_images", "google_search_images", {"q": "apple inc", "num": 5}),
        ("news", "serper_news", "google_search_news", {"q": "apple inc", "num": 5}),
        ("places", "serper_places", "google_search_places", {"q": "coffee", "location": "Dhaka,Bangladesh"}),
        ("maps", "serper_maps", "google_search_maps", {"q": "coffee", "ll": "@23.8103,90.4125,14z"}),
        ("videos", "serper_videos", "google_search_videos", {"q": "rust tutorial", "num": 5}),
        ("shopping", "serper_shopping", "google_search_shopping", {"q": "mechanical keyboard", "num": 5}),
        ("scholar", "serper_scholar", "google_search_scholar", {"q": "large language models", "num": 5}),
        ("patents", "serper_patents", "google_search_patents", {"q": "wireless charging", "num": 5}),
        ("autocomplete", "serper_autocomplete", "google_search_autocomplete", {"q": "apple in"}),
        ("lens", "serper_lens", "google_search_lens", {"url": "https://www.wikipedia.org/portal/wikipedia.org/assets/img/Wikipedia-logo-v2.png"}),
        ("scrape", "serper_scrape", "webpage_scrape", {"url": "https://example.com"}),
    ]

    report: dict = {
        "tooling": {
            "ours": {
                "startup_ms": round(ours.startup_ms, 1),
                "tools_ms": round(ours.tools_ms, 1),
                "tools_bytes": ours.tools_bytes,
                "tools": len(ours.tools),
                "idle_rss_kb": ours.idle_rss_kb,
                "cpu_ms_at_idle": ours.cpu_ms,
            },
            "python": None
            if not python
            else {
                "startup_ms": round(python.startup_ms, 1),
                "tools_ms": round(python.tools_ms, 1),
                "tools_bytes": python.tools_bytes,
                "tools": len(python.tools),
                "idle_rss_kb": python.idle_rss_kb,
                "cpu_ms_at_idle": python.cpu_ms,
            },
        },
        "calls": [],
        "only_ours": sorted(ours_names - python_names),
        "only_python": sorted(python_names - ours_names),
    }

    cid = None
    for label, our_tool, python_tool, arguments in cases:
        entry: dict = {"name": label, "arguments": arguments}
        ours_result = ours.call(label, our_tool, arguments)
        entry["ours"] = {
            "latency_ms": round(ours_result.latency_ms, 1),
            "bytes": ours_result.bytes,
            "is_error": ours_result.is_error,
            **summarize_ours(ours_result),
        }
        if label == "places":
            rows = structured(ours_result).get("results") or []
            for row in rows:
                if isinstance(row, dict) and row.get("cid"):
                    cid = row["cid"]
                    break
        if python:
            python_result = python.call(label, python_tool, arguments)
            entry["python"] = {
                "latency_ms": round(python_result.latency_ms, 1),
                "bytes": python_result.bytes,
                "is_error": python_result.is_error,
                **summarize_python(python_result),
            }
        report["calls"].append(entry)

    reviews_arguments = {"cid": cid} if cid else {"placeId": "ChIJL6wWn9nKVTcR"}
    ours_reviews = ours.call("reviews", "serper_reviews", reviews_arguments)
    report["calls"].append(
        {
            "name": "reviews",
            "arguments": reviews_arguments,
            "ours": {
                "latency_ms": round(ours_reviews.latency_ms, 1),
                "bytes": ours_reviews.bytes,
                "is_error": ours_reviews.is_error,
                **summarize_ours(ours_reviews),
            },
            "python": None
            if not python
            else _python_reviews(python, reviews_arguments),
        }
    )

    for label, tool, arguments in [
        ("locations", "serper_locations", {"q": "Dhaka", "limit": 3}),
        ("health", "serper_health", {}),
        ("session", "serper_session", {}),
        ("batch", "serper_search_batch", {"kind": "search", "queries": [{"q": "apple inc"}, {"q": "google"}]}),
    ]:
        result = ours.call(label, tool, arguments)
        report["calls"].append(
            {
                "name": label,
                "arguments": arguments,
                "ours": {
                    "latency_ms": round(result.latency_ms, 1),
                    "bytes": result.bytes,
                    "is_error": result.is_error,
                    **summarize_ours(result),
                },
                "python": None,
            }
        )

    report["load"] = {"ours": load_test(ours, "serper_search", "rust benchmark", args.load)}
    if python:
        report["load"]["python"] = load_test(python, "google_search", "rust benchmark", args.load)

    cpu_after = ours.cpu_ms
    report["tooling"]["ours"]["cpu_ms_after_all_calls"] = cpu_after
    if python:
        report["tooling"]["python"]["cpu_ms_after_all_calls"] = python.cpu_ms

    ours.stop()
    if python:
        python.stop()

    Path(args.json_out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.json_out).write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
