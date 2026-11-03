# Two-replica forced-failover rig (§12 task 5)

Measures the **leader handoff** of the OSS HA design under load: how fast a standby
replica takes over when the leader dies, and how many duplicate outputs the at-least-once
redelivery produces. It is the minimal, honest version of §12 task 5 — a **leader-gated
publisher** built from the proven active-flow primitive (`examples/leader_probe.rs`), with
**no Feldera and no full sidecar**. Feldera reconstitution is proven separately
(`OSS-HA-DR-DESIGN.md` §3.1); this isolates the handoff, which is a property of the leader
gate and the broker's exclusive-queue semantics, not of Feldera.

## How it works

One **exclusive** queue `leader-q` is simultaneously the leader lock and the work queue:
the broker delivers messages only to the currently-Active flow, so the standby replica is
bound but starved until promoted. `active_flow_indication` reports the promotion instant;
guaranteed delivery with **ack-after-publish** makes the at-least-once overlap measurable.

```
load_gen ──persistent──> topic failover/in
                              │ (queue subscription)
                         [ leader-q, exclusive ]
                          │ only the Active flow receives
        ┌─────────────────┴─────────────────┐
   replica A                            replica B
   (gate: publish only while Active)    (bound, starved until promoted)
        └──────── failover/out ─────────────┘
                     │
               output_recorder ──> /tmp/failover-out.jsonl ──> analyze.py
```

Each replica turns every input into a per-key rolling-count row published to `failover/out`,
**then acks** the input. Kill the leader mid-load; the standby is promoted and resumes.

## Run

Requires a running Solace broker (SMF `localhost:55555`, SEMP `localhost:8089`, admin/admin
by default — matches the `continuum-payments-feldera` stack). The example binaries connect
as `default`/`default`.

```sh
# graceful (SIGTERM → clean flow unbind) — the rolling-upgrade case
SOLACE_BROKER_PASSWORD=default bash failover-rig/run.sh graceful

# ungraceful (SIGKILL → broker detects the lost flow, redelivers unacked)
SOLACE_BROKER_PASSWORD=default bash failover-rig/run.sh ungraceful
```

Tunables (env): `RATE` (msg/s, 200), `DURATION_S` (20), `KEYS` (8), `KILL_AT_S` (DURATION/2).
`provision.sh` (idempotent) creates the exclusive `leader-q` + its `failover/in` subscription.

## The three numbers

`analyze.py` reports, measured at the publishers' own `pub_ts`:

- **FAILOVER GAP** — `first_new_leader_publish − last_old_leader_publish`.
- **OVERLAP DUPLICATES** — `input_seq`s published by **both** replicas = the at-least-once
  redelivery (the leader published them but died before acking; they are redelivered to the
  standby). Downstream RGMID dedup (design §3.7) removes these.
- **SPLIT-BRAIN (temporal)** — any input_seq where the two leaders' publish windows overlap
  in time. Expected **none** on a single exclusive queue (the broker serialises the handoff).

It also prints **keyed-count resets** — how many keys' rolling counts jumped backward because
the new leader rebuilt in-memory count from 0. That is exactly the keyed-state loss the
sidecar's external store (§3.5 / Delta-on-S3 §3.8) removes, shown here as the untreated case.

## Measured (3 runs each, 100 msg/s, 8 keys, local broker)

| Scenario              | Gap          | Overlap duplicates | Split-brain |
|-----------------------|--------------|--------------------|-------------|
| Graceful (SIGTERM)    | **9–13 ms**  | **0**              | none        |
| Ungraceful (SIGKILL)  | **3–15 ms**  | **47–67**          | none        |

The graceful handoff is clean and duplicate-free — the rolling-upgrade / drain case the HA
design sells. The ungraceful handoff is just as fast (the dead leader stops publishing
instantly) but redelivers its in-flight unacked window (~0.5 s × rate ≈ 50 rows here) as
duplicates — the expected at-least-once cost, bounded and dedup-able.

## Caveats (do not oversell the number)

- **Handoff, not end-to-end.** This measures first new-leader **publish**, not first new-leader
  **snapshot**. A real Feldera replica also reconstitutes state before its first correct
  output; that adds the §3.4 pause/resume stall (task 9) and the reconstitution already
  proven in §3.1. The keyed-count resets above are the visible stand-in for that gap.
- **SIGKILL detection was *not* masked.** §12 task 1 warned a local containerised SIGKILL can
  be hidden by docker port-forward. Here the replicas are **host processes** on a real socket,
  so a `kill -9` closes TCP immediately and the broker promotes in single-digit ms — better
  than the keepalive-bound worst case. A containerised replica (real RST on kill) is the
  follow-up if a container-faithful number is needed.
- **Single broker.** True split-brain needs a broker HA pair; on one broker the exclusive
  queue serialises the handoff by construction, so the 0 split-brain result is expected, not
  a stress test of concurrent leaders.
