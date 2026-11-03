#!/usr/bin/env python3
"""Compare the `rolling` view of two Feldera replicas and report divergence.

Each argument is a file holding one Feldera /query response (newline-delimited JSON
rows like {"key":"k0","cnt":12}). Prints per-key max divergence and whether the two
replicas are in lockstep (identical key→cnt maps).

Run: python3 -I compare.py <a.json> <b.json>
"""
import json
import sys


def load(path):
    d = {}
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                r = json.loads(line)
                d[str(r["k"])] = int(r["cnt"])
            except (ValueError, KeyError):
                pass
    return d


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: compare.py <a.json> <b.json>", file=sys.stderr)
        return 2
    a = load(sys.argv[1])
    b = load(sys.argv[2])
    keys = sorted(set(a) | set(b))
    total_a = sum(a.values())
    total_b = sum(b.values())
    max_div = 0
    diffs = []
    for k in keys:
        va, vb = a.get(k, 0), b.get(k, 0)
        d = abs(va - vb)
        if d > max_div:
            max_div = d
        if d != 0:
            diffs.append((k, va, vb))

    identical = (a == b)
    print(f"A: {len(a)} keys, total {total_a} | B: {len(b)} keys, total {total_b}")
    print(f"max per-key divergence: {max_div}")
    if identical:
        print("LOCKSTEP: identical (both replicas agree on every key)")
    else:
        print(f"DIVERGENT on {len(diffs)} key(s): " +
              ", ".join(f"{k}(A={va},B={vb})" for k, va, vb in diffs[:12]) +
              (" …" if len(diffs) > 12 else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
