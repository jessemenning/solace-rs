#!/usr/bin/env python3
"""Analyze the two-replica failover recording (§12 task 5).

Input: a JSONL file whose lines are `<recv_ms> <output-json>`, where output-json is one
row published by a replica: {replica, input_seq, key, output_seq, count, pub_ts, redelivered}.

Reports, measured at the publishers' own pub_ts (not the recorder's clock):
  - gap_ms      : first_new_leader_pub - last_old_leader_pub  (the failover publish gap)
  - overlap     : input_seq values published by BOTH replicas (at-least-once redelivery)
  - temporal    : whether the two leaders' publish windows overlapped in time (split-brain
                  signal; expected 0 on a single exclusive queue)

Run: python3 -I analyze.py <out.jsonl> [initial_leader_id]
"""
import json
import sys
from collections import defaultdict


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: analyze.py <out.jsonl> [initial_leader_id]", file=sys.stderr)
        return 2
    path = sys.argv[1]
    hint_leader = sys.argv[2] if len(sys.argv) > 2 else None

    rows = []
    bad = 0
    with open(path) as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            # split off the recorder's recv_ms prefix, parse the JSON remainder
            sp = line.find(" ")
            payload = line[sp + 1:] if sp > 0 else line
            try:
                r = json.loads(payload)
                r["input_seq"] = int(r["input_seq"])
                r["pub_ts"] = int(r["pub_ts"])
                rows.append(r)
            except (ValueError, KeyError):
                bad += 1

    if not rows:
        print(f"no rows parsed from {path} ({bad} malformed)")
        return 1

    by_replica = defaultdict(list)
    for r in rows:
        by_replica[r["replica"]].append(r)

    replicas = sorted(by_replica)
    print(f"rows: {len(rows)} total, {bad} malformed | replicas that published: {replicas}")
    for rep in replicas:
        rs = by_replica[rep]
        ts = [r["pub_ts"] for r in rs]
        seqs = {r["input_seq"] for r in rs}
        print(f"  {rep}: {len(rs)} rows | pub_ts {min(ts)}..{max(ts)} "
              f"| input_seq {min(seqs)}..{max(seqs)} | redelivered={sum(1 for r in rs if r.get('redelivered'))}")

    if len(replicas) < 2:
        print("\nNO TAKEOVER: only one replica ever published — failover did not occur "
              "(or the standby never received). gap/overlap undefined.")
        return 0

    # Identify old vs new leader. Prefer the hint; else the replica that started first.
    starts = {rep: min(r["pub_ts"] for r in by_replica[rep]) for rep in replicas}
    if hint_leader in by_replica:
        old = hint_leader
    else:
        old = min(starts, key=starts.get)
    new = [r for r in replicas if r != old][0]

    last_old = max(r["pub_ts"] for r in by_replica[old])
    first_new = min(r["pub_ts"] for r in by_replica[new])
    gap_ms = first_new - last_old

    old_seqs = {r["input_seq"] for r in by_replica[old]}
    new_seqs = {r["input_seq"] for r in by_replica[new]}
    overlap = old_seqs & new_seqs

    # Temporal overlap = new leader published before the old leader's last publish.
    temporal = first_new < last_old
    temporal_seqs = sorted(
        s for s in overlap
        if min(r["pub_ts"] for r in by_replica[new] if r["input_seq"] == s)
        <= max(r["pub_ts"] for r in by_replica[old] if r["input_seq"] == s)
    ) if overlap else []

    print()
    print(f"old leader = {old}, new leader = {new}")
    print(f"last old-leader publish : {last_old} ms")
    print(f"first new-leader publish: {first_new} ms")
    print(f"--> FAILOVER GAP        : {gap_ms} ms"
          + ("  (NEGATIVE = publish windows overlapped in time)" if gap_ms < 0 else ""))
    print(f"--> OVERLAP DUPLICATES  : {len(overlap)} input_seq republished by both"
          + (f" (seqs {min(overlap)}..{max(overlap)})" if overlap else ""))
    print(f"--> SPLIT-BRAIN (temporal): {'YES — ' + str(len(temporal_seqs)) + ' seqs' if temporal else 'none'}"
          " (expected none on a single exclusive queue)")

    # Per-key continuity: did any key's rolling count go backwards across the handoff?
    # (The new leader rebuilds count from 0, so a drop is expected without external state —
    # this quantifies exactly the keyed-state loss the sidecar's §3.5 externalisation fixes.)
    resets = 0
    last_count = {}
    for r in sorted(rows, key=lambda r: r["pub_ts"]):
        k = r["key"]
        c = int(r.get("count", 0))
        if k in last_count and c < last_count[k]:
            resets += 1
        last_count[k] = c
    print(f"--> keyed-count resets  : {resets} (new leader rebuilds per-key count from 0; "
          "this is the state loss §3.5 external store removes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
