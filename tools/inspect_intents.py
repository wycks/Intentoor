
#!/usr/bin/env python3
"""Inspect ingestor JSONL intent logs.

Examples:
  python tools/inspect_intents.py --stats
  python tools/inspect_intents.py --source cowswap --head 3 --show normalized
  python tools/inspect_intents.py --source uniswapx --contains-id 0xabc --show raw --pretty

If --file isn't provided, the newest file in ./out/ingestor is used.
"""

from __future__ import annotations

import argparse
import json
import os
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Iterable, Optional


# Minimal token registry for nicer local inspection (no RPC/API calls).
# Extend as needed.
KNOWN_TOKENS: dict[str, dict[str, tuple[str, int]]] = {
    "ethereum-mainnet": {
        "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48": ("USDC", 6),
        "0xdac17f958d2ee523a2206206994597c13d831ec7": ("USDT", 6),
        "0x6b175474e89094c44da98b954eedeac495271d0f": ("DAI", 18),
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2": ("WETH", 18),
        "0x2260fac5e5542a773aa44fbcfedf7c193bc2c599": ("WBTC", 8),
        "0xdef1ca1fb7fbcdc777520aa7f396b4e015f497ab": ("CoW", 18),
    }
}


def newest_jsonl(out_dir: Path) -> Path:
    if not out_dir.exists():
        raise FileNotFoundError(f"out dir not found: {out_dir}")
    candidates = sorted(out_dir.glob("*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not candidates:
        raise FileNotFoundError(f"no .jsonl files in: {out_dir}")
    return candidates[0]


def iter_jsonl(path: Path) -> Iterable[dict[str, Any]]:
    with path.open("r", encoding="utf-8") as f:
        for idx, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError as e:
                raise ValueError(f"invalid json at {path}:{idx}: {e}") from e


def short(s: str, n: int = 14) -> str:
    if len(s) <= n:
        return s
    return s[:8] + "…" + s[-6:]


def norm_addr(s: str) -> str:
    s = (s or "").strip()
    return s.lower()


def resolve_token(network: str, addr: str) -> tuple[Optional[str], Optional[int]]:
    net = (network or "").strip()
    reg = KNOWN_TOKENS.get(net, {})
    sym_dec = reg.get(norm_addr(addr))
    if not sym_dec:
        return None, None
    return sym_dec


def format_amount(raw_amount: str, decimals: Optional[int]) -> str:
    """Best-effort fixed-point rendering without float rounding."""
    raw_amount = (raw_amount or "").strip()
    if not raw_amount:
        return ""
    if decimals is None:
        return raw_amount
    if not raw_amount.isdigit():
        return raw_amount

    if decimals == 0:
        return raw_amount

    if len(raw_amount) <= decimals:
        frac = raw_amount.rjust(decimals, "0")
        frac = frac.rstrip("0")
        return f"0.{frac}" if frac else "0"

    whole = raw_amount[:-decimals]
    frac = raw_amount[-decimals:].rstrip("0")
    return f"{whole}.{frac}" if frac else whole


def to_iso(ts: Any) -> str:
    if isinstance(ts, str):
        return ts
    return ""


@dataclass
class Stats:
    total: int = 0
    by_source: dict[str, int] = None  # type: ignore

    def __post_init__(self) -> None:
        if self.by_source is None:
            self.by_source = {}

    def add(self, msg: dict[str, Any]) -> None:
        self.total += 1
        src = str(msg.get("source", ""))
        self.by_source[src] = self.by_source.get(src, 0) + 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--file", type=str, help="Path to JSONL file (defaults to newest in ./out/ingestor)")
    ap.add_argument("--source", type=str, help="Filter by envelope.source (e.g. cowswap, uniswapx)")
    ap.add_argument("--contains-id", type=str, help="Filter where envelope.id contains substring")
    ap.add_argument("--head", type=int, default=20, help="Max rows to print")
    ap.add_argument("--stats", action="store_true", help="Print only statistics")
    ap.add_argument(
        "--show",
        choices=["summary", "normalized", "raw", "envelope"],
        default="summary",
        help="What to print for each row",
    )
    ap.add_argument("--pretty", action="store_true", help="Pretty-print JSON for --show raw/normalized/envelope")
    ap.add_argument(
        "--human",
        action="store_true",
        help="In summary view, render amounts using known token decimals when available",
    )

    args = ap.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    default_out_dir = repo_root / "out" / "ingestor"

    path = Path(args.file) if args.file else newest_jsonl(default_out_dir)

    stats = Stats()
    printed = 0

    for msg in iter_jsonl(path):
        stats.add(msg)

        if args.stats:
            continue

        if args.source and msg.get("source") != args.source:
            continue

        msg_id = str(msg.get("id", ""))
        if args.contains_id and args.contains_id not in msg_id:
            continue

        if printed >= args.head:
            continue

        if args.show == "summary":
            emitted = to_iso(msg.get("emitted_at"))
            src = str(msg.get("source", ""))
            net = str(msg.get("network", ""))
            n = msg.get("normalized") or {}

            sell_addr = str(n.get("sell_token", ""))
            buy_addr = str(n.get("buy_token", ""))

            sell_sym, sell_dec = resolve_token(net, sell_addr)
            buy_sym, buy_dec = resolve_token(net, buy_addr)

            sell_label = f"{sell_sym}:{short(sell_addr)}" if sell_sym else short(sell_addr)
            buy_label = f"{buy_sym}:{short(buy_addr)}" if buy_sym else short(buy_addr)

            sell_amt_raw = str(n.get("sell_amount", ""))
            min_buy_raw = str(n.get("min_buy_amount", ""))
            sell_amt = format_amount(sell_amt_raw, sell_dec) if args.human else sell_amt_raw
            min_buy = format_amount(min_buy_raw, buy_dec) if args.human else min_buy_raw
            deadline = n.get("deadline_unix")
            deadline_s = str(deadline) if deadline is not None else ""
            print(
                f"{emitted}  {src:8}  {net:16}  id={short(msg_id):16}  "
                f"{sell_label:20}->{buy_label:20} sell={sell_amt} minbuy={min_buy} deadline={deadline_s}"
            )
        else:
            payload: Any
            if args.show == "normalized":
                payload = msg.get("normalized")
            elif args.show == "raw":
                payload = msg.get("raw")
            else:
                payload = msg

            if args.pretty:
                print(json.dumps(payload, indent=2, sort_keys=True))
            else:
                print(json.dumps(payload, separators=(",", ":")))

        printed += 1

    if args.stats:
        print(f"file: {path}")
        print(f"total: {stats.total}")
        for k in sorted(stats.by_source.keys()):
            print(f"source[{k or '(missing)'}]: {stats.by_source[k]}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
