# Two-Feldera warm-replica lockstep proof (§12 task 5, Capability 2)

Proves the untested half of Capability 2: **two warm Feldera replicas fed the same stream
maintain identical view state**, so a failover between them has no state reset. The pure
leader handoff (gap + duplicates) is measured separately by the parent `failover-rig/`; this
isolates warm redundancy. No `solace_output` — the views are read back over REST `/query`,
so there is no changelog-retraction handling.

## How it works

```
load_gen (KEYED) ──persistent──> failover/in/k0 .. k7
                                      │  (both queues subscribe failover/in/>)
                 ┌────────────────────┴────────────────────┐
             queue fq-a                                 queue fq-b
                 │                                          │
          Feldera A (:8091)                          Feldera B (:8092)
          view rolling = COUNT(*) GROUP BY k         (same program)
                 └──────── REST /query ─────────── compare.py ──> lockstep?
```

Each replica ingests the **same** keyed stream through its **own** non-exclusive queue (a
topic fan-out gives each queue a full copy), so both stay warm independently. `compare.py`
diffs the two `rolling` views during load and after drain.

## Run

Requires the `continuum-payments` broker up (SMF `localhost:55555`, SEMP `localhost:8089`,
docker network `continuum-payments-feldera_default`) and the `load_gen` example built
(`cargo build --example load_gen`). Starts two fresh Feldera containers and removes them on
exit (`KEEP=1` to leave them for inspection).

```sh
bash failover-rig/feldera-lockstep/run.sh
```

Env: `RATE` (150), `DURATION_S` (25), `KEYS` (8), `PORT_A`/`PORT_B` (8091/8092).

## Measured

| Phase            | Result                                                        |
|------------------|---------------------------------------------------------------|
| During load      | 8 keys tracked; per-key divergence **≤ 1** (one replica a msg or two ahead — in-flight skew) |
| After drain (4 s)| 8 keys, totals equal, **max divergence 0 — LOCKSTEP identical** |

The ≤1 transient skew is exactly the per-partition batching skew §4 notes the keyed-upsert
model absorbs: the replicas are at most one in-flight message apart and converge the instant
input stops. Combined with the parent rig's ~10 ms handoff, this is the evidence for
Capability 2's "survive a crash with no gap": the standby is already at the leader's state,
so promotion costs only the handoff, not a rebuild.

## Gotcha found (worth keeping)

`solace_input` `topic_pattern` captures are **not** name-matched to columns. A capture `{k}`
is exposed via `CONNECTOR_METADATA()['k']` and must be pulled into a column with an explicit
`DEFAULT CAST(CONNECTOR_METADATA()['k'] AS VARCHAR)` — a bare `k VARCHAR` column stays NULL
and every row collapses into one group (the aggregate total is still correct, which hides
the bug). `solace_topic`/`broker_ts` additionally need `include_topic`/`include_broker_timestamp`;
pattern captures need no flag. See `program.sql`.

## Scope

Proves warm-replica convergence only. It does **not** gate egress or kill a replica — that
is the parent `failover-rig/` handoff measurement, and the full end-to-end integration (gate
relaying real Feldera output through a kill) is the remaining Capability-2 step.
