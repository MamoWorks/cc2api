#!/usr/bin/env python3
"""请求路径打点分析脚本。

假设 gateway 已启动并把 stdout/stderr 重定向到 --log 指定的文件，例如：
    PERF_TRACE=1 ./target/release/claude-code-gateway > /tmp/gw.log 2>&1 &

脚本会：
1. 串行向 --url 发 --runs 次 POST 请求，每次校验 HTTP 200
2. 从 --log 读取请求期间新增的 perf: 日志行
3. 把每个 rid 的各阶段耗时聚合成中位数/min/max 表

仅标准库依赖，Python 3.8+。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Optional

ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")
PERF_RE = re.compile(
    r"INFO\s+perf:\s+rid=(?P<rid>\w+)\s+phase=(?P<phase>\S+)\s+ms=(?P<ms>[\d.]+)\s*$"
)

DEFAULT_BODY = {
    "model": "claude-haiku-4-5",
    "max_tokens": 16,
    "messages": [{"role": "user", "content": "hi"}],
}


def send_one(url: str, token: str, body: bytes, timeout: float) -> tuple[int, bytes]:
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "anthropic-version": "2023-06-01",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read() or b""


def parse_perf_lines(text: str) -> list[dict]:
    events: list[dict] = []
    for raw in text.splitlines():
        line = ANSI_RE.sub("", raw)
        m = PERF_RE.search(line)
        if not m:
            continue
        events.append(
            {
                "rid": m.group("rid"),
                "phase": m.group("phase"),
                "ms": float(m.group("ms")),
            }
        )
    return events


def aggregate(events: list[dict]) -> dict:
    by_rid: dict[str, list[dict]] = {}
    for e in events:
        by_rid.setdefault(e["rid"], []).append(e)

    phase_samples: dict[str, list[float]] = {}
    totals: list[float] = []
    rids_with_total: list[str] = []
    for rid, evs in by_rid.items():
        has_total = False
        for e in evs:
            if e["phase"] == "total":
                totals.append(e["ms"])
                has_total = True
                continue
            phase_samples.setdefault(e["phase"], []).append(e["ms"])
        if has_total:
            rids_with_total.append(rid)

    rows = []
    for phase, samples in phase_samples.items():
        rows.append(
            {
                "phase": phase,
                "median": statistics.median(samples),
                "min": min(samples),
                "max": max(samples),
                "count": len(samples),
            }
        )
    rows.sort(key=lambda r: r["median"], reverse=True)
    return {
        "rows": rows,
        "total_median": statistics.median(totals) if totals else 0.0,
        "total_min": min(totals) if totals else 0.0,
        "total_max": max(totals) if totals else 0.0,
        "rids": rids_with_total,
        "rid_count": len(by_rid),
    }


def render(agg: dict) -> str:
    rows = agg["rows"]
    if not rows:
        return "[no perf: log lines observed — is PERF_TRACE=1 set and --log pointing at the gateway output?]"
    width = max(max(len(r["phase"]) for r in rows), len("phase"))
    total_med = agg["total_median"]

    def fmt_ms(x: float) -> str:
        return f"{x:9.2f} ms"

    lines = []
    header = f"    {'phase':<{width}}  {'median':>12}  {'min':>12}  {'max':>12}  {'share':>6}"
    lines.append(header)
    lines.append("    " + "-" * (len(header) - 4))
    for i, r in enumerate(rows):
        marker = ">>> " if i == 0 else "    "
        share = (r["median"] / total_med * 100) if total_med > 0 else 0.0
        lines.append(
            f"{marker}{r['phase']:<{width}}  {fmt_ms(r['median'])}  {fmt_ms(r['min'])}  {fmt_ms(r['max'])}  {share:5.1f}%"
        )
    lines.append(
        f"    {'total':<{width}}  {fmt_ms(total_med)}  {fmt_ms(agg['total_min'])}  {fmt_ms(agg['total_max'])}"
    )
    lines.append(
        f"    (perf events from {len(agg['rids'])}/{agg['rid_count']} rids with total)"
    )
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description="cc-bridge request-path profiler")
    ap.add_argument("--log", required=True, help="gateway stdout+stderr capture file")
    ap.add_argument("--token", required=True, help="API token (sk_...)")
    ap.add_argument(
        "--url",
        default="http://127.0.0.1:5674/v1/messages",
        help="gateway endpoint",
    )
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--timeout", type=float, default=60.0, help="per-request timeout")
    ap.add_argument("--model", default=None, help="override body.model")
    ap.add_argument("--prompt", default=None, help="override user message content")
    ap.add_argument("--max-tokens", type=int, default=None)
    ap.add_argument("--body-file", default=None, help="json body file (overrides all others)")
    ap.add_argument("--json", dest="json_out", default=None)
    ap.add_argument(
        "--flush-wait",
        type=float,
        default=0.2,
        help="seconds to wait after each request for log flush",
    )
    args = ap.parse_args()

    log_path = Path(args.log)
    if not log_path.exists():
        print(f"[error] log file does not exist: {log_path}", file=sys.stderr)
        return 2

    if args.body_file:
        body_obj = json.loads(Path(args.body_file).read_text())
    else:
        body_obj = dict(DEFAULT_BODY)
        if args.model:
            body_obj["model"] = args.model
        if args.max_tokens is not None:
            body_obj["max_tokens"] = args.max_tokens
        if args.prompt:
            body_obj["messages"] = [{"role": "user", "content": args.prompt}]
    body_bytes = json.dumps(body_obj).encode("utf-8")

    start_offset = log_path.stat().st_size
    results: list[tuple[int, int]] = []
    t_start = time.time()

    for i in range(args.runs):
        t0 = time.time()
        status, resp_body = send_one(args.url, args.token, body_bytes, args.timeout)
        dt = time.time() - t0
        print(
            f"[req {i + 1}/{args.runs}] status={status} wall={dt * 1000:.1f}ms",
            flush=True,
        )
        results.append((status, len(resp_body)))
        if status != 200:
            snippet = resp_body[:400].decode("utf-8", errors="replace")
            print(f"         body: {snippet}", file=sys.stderr)
        time.sleep(args.flush_wait)

    ok_runs = sum(1 for s, _ in results if s == 200)
    if ok_runs == 0:
        print("[error] no 200 responses; aborting analysis", file=sys.stderr)
        return 1

    time.sleep(args.flush_wait)
    with log_path.open("rb") as f:
        f.seek(start_offset)
        blob = f.read()
    text = blob.decode("utf-8", errors="replace")
    events = parse_perf_lines(text)
    agg = aggregate(events)

    print()
    print(render(agg))
    print(
        f"    (HTTP: {ok_runs}/{args.runs} were 200, total wall {(time.time() - t_start):.1f}s)"
    )

    if args.json_out:
        Path(args.json_out).write_text(
            json.dumps(
                {
                    "events": events,
                    "aggregate": agg,
                    "results": [{"status": s, "body_bytes": n} for s, n in results],
                },
                indent=2,
                ensure_ascii=False,
            )
        )
        print(f"\n[json] wrote {args.json_out}")

    return 0 if ok_runs == args.runs else 1


if __name__ == "__main__":
    sys.exit(main())
