#!/usr/bin/env python3
"""Analyze the egress-gate downstream recording (§12 task 5, end-to-end Capability 2).

The recorder writes `<recv_ms> <topic> <json>`, one line per downstream message. The topic
(failover/out/a | failover/out/b) names the relaying gate; the JSON is the Feldera `rolling`
output (k, cnt), possibly wrapped in an insert/delete envelope.

Proves the Capability-2 payoff: after the leader gate dies, the standby gate relays its
already-warm replica's output, so downstream continues from the CORRECT per-key count with
NO reset and only the lock-handoff gap.

Run: python3 -I analyze.py <out.jsonl>
"""
import re
import sys
from collections import defaultdict

K_RE = re.compile(r'"k"\s*:\s*"([^"]+)"')
CNT_RE = re.compile(r'"cnt"\s*:\s*(\d+)')


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: analyze.py <out.jsonl>", file=sys.stderr)
        return 2

    # A materialized-view solace_output emits a changelog: inserts carry the authoritative
    # latest (k,cnt); deletes and empty tombstones are retractions we intentionally ignore
    # (the design's downstream dedup/latest-wins collapses them — §3.8/§4).
    rows = []  # (recv_ms, gate, k, cnt)  -- inserts only
    retractions = 0
    with open(sys.argv[1]) as fh:
        for line in fh:
            parts = line.strip().split(" ", 2)
            payload = parts[2] if len(parts) == 3 else ""
            km = K_RE.search(payload)
            cm = CNT_RE.search(payload)
            if ('"delete"' in payload) or not (km and cm):
                retractions += 1
                continue
            gate = parts[1].rsplit("/", 1)[-1]  # 'a' or 'b'
            rows.append((int(parts[0]), gate, km.group(1), int(cm.group(1))))

    if not rows:
        print(f"no insert rows parsed ({retractions} retraction/tombstone rows)")
        return 1
    rows.sort(key=lambda r: r[0])
    print(f"(ignored {retractions} MV changelog retraction/tombstone rows; using insert rows only)")

    gates = sorted({r[1] for r in rows})
    per_gate = defaultdict(int)
    for _, g, _, _ in rows:
        per_gate[g] += 1
    print(f"insert rows: {len(rows)} | relayed by gate: "
          + ", ".join(f"{g}={per_gate[g]}" for g in gates))

    if len(gates) < 2:
        print("NO HANDOFF: only one gate relayed — failover did not occur downstream.")
        return 0

    old = rows[0][1]
    new = [g for g in gates if g != old][0]
    last_old_ms = max(r[0] for r in rows if r[1] == old)
    new_after = [r for r in rows if r[1] == new and r[0] >= last_old_ms]
    first_new_ms = min((r[0] for r in new_after), default=None)
    gap = (first_new_ms - last_old_ms) if first_new_ms is not None else None

    # Last count each key reached on the OLD gate, and the first count the NEW gate relays
    # for that key after the handoff. A reset = new first < old last.
    last_old_cnt = {}
    for recv_ms, g, k, cnt in rows:
        if g == old:
            last_old_cnt[k] = cnt  # rows are time-sorted, so this ends as the last
    first_new_cnt = {}
    for recv_ms, g, k, cnt in new_after:
        if k not in first_new_cnt:
            first_new_cnt[k] = cnt

    resets = []
    continued = 0
    for k, old_c in last_old_cnt.items():
        if k in first_new_cnt:
            if first_new_cnt[k] < old_c:
                resets.append((k, old_c, first_new_cnt[k]))
            else:
                continued += 1

    final_cnt = {}
    for _, _, k, cnt in rows:
        final_cnt[k] = max(final_cnt.get(k, 0), cnt)

    print()
    print(f"old leader gate = {old}, new leader gate = {new}")
    print(f"--> FAILOVER GAP (downstream) : {gap} ms"
          if gap is not None else "--> new gate never relayed after handoff")
    print(f"--> STATE RESET across handoff: "
          + (f"NONE — {continued} key(s) continued from the correct count"
             if not resets else
             f"{len(resets)} key(s) RESET: " + ", ".join(f"{k}({o}->{n})" for k, o, n in resets[:8])))
    print(f"--> final per-key counts      : "
          + ", ".join(f"{k}={final_cnt[k]}" for k in sorted(final_cnt)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
