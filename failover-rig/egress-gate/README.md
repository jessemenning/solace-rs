# End-to-end egress-gate failover (§12 task 5, Capability 2 complete)

The final Capability-2 piece: two **warm** Feldera replicas each publish their `rolling`
view, and a per-replica **egress gate** relays only the **leader's** output downstream. Kill
the leader gate mid-load and the standby — whose replica is already at the correct state
(see `../feldera-lockstep/`) — takes over. Proves the payoff: the downstream stream
continues **with no per-key count reset** and only the lock-handoff gap.

This closes the loop the other two rigs opened: `../` measured the bare leader handoff,
`../feldera-lockstep/` proved two real Feldera replicas stay identical, and this wires them
together on real Feldera output.

## Architecture

```
load_gen (KEYED) ─persistent─> failover/in/k0..k7
        ┌──────────────── both input queues subscribe failover/in/> ──────────────┐
   Feldera A (:8091)                                                     Feldera B (:8092)
   rolling → solace_output                                               rolling → solace_output
        │ failover/fout/a/{k}                                                 │ failover/fout/b/{k}
     gate A  ── leader? ──┐                                       ┌── leader? ──  gate B
        │ relay if Active │   exclusive eg-leader-q (lock only)   │ relay if Active │
        └───────────> failover/out/a          failover/out/b <────┘
                              └──── recorder (failover/out/>) ────┘ ──> analyze.py
```

Both gates bind the exclusive `eg-leader-q` for election only (no work flows through it).
The leader gate relays its replica's Feldera output verbatim; the standby drops its
replica's (identical) output on the floor until promoted.

## Run

Requires the `continuum-payments` broker (SMF `localhost:55555`, SEMP `localhost:8089`,
network `continuum-payments-feldera_default`). Starts two fresh Feldera containers, removed
on exit (`KEEP=1` to keep). Env: `RATE` (120), `DURATION_S` (30), `KEYS` (8), `KILL_AT_S`.

```sh
bash failover-rig/egress-gate/run.sh graceful     # SIGTERM the leader gate (clean unbind)
bash failover-rig/egress-gate/run.sh ungraceful   # SIGKILL the leader gate
```

## Measured (120 msg/s, 8 keys, local broker)

| Scenario              | Downstream gap | State reset across handoff |
|-----------------------|----------------|----------------------------|
| Graceful (SIGTERM)    | **~44 ms**     | **NONE** — all 8 keys continued from the correct count |
| Ungraceful (SIGKILL)  | **~267 ms**    | **NONE** — all 8 keys continued |

The **no-reset** result is the whole point: because the standby replica was warm, the new
leader gate relays output that is *already* at the right per-key count — downstream never
sees a counter jump backwards. This is Capability 2's "survive a crash with no gap," proven
end to end on real Feldera OSS.

The gap is **not** the lock takeover — the active-flow promotion itself is the ~3 ms measured
in `../` — it is dominated by the sink's `dedup_window_ms` (300 ms here): the new leader's
first downstream output is held up to one conflation window. Shrink the window to shrink the
gap; the ceiling is `dedup_window + lock_takeover`, and there is no rebuild in it because the
replica is warm.

## Reading the changelog

A materialized-view `solace_output` emits a **changelog**: `{"insert":{"k":..,"cnt":..}}`
rows carry the authoritative latest value, interleaved with `{"delete":...}` retractions and
empty tombstones (conflated by `dedup_window_ms`). `analyze.py` uses the insert rows and
ignores the retractions — exactly as the design's downstream latest-wins dedup does
(§3.8/§4). Per run, expect roughly as many retraction rows as inserts; that is normal MV
output, not loss.

## Scope / caveat

Measures the handoff-to-first-**publish** with warm replicas — it does **not** add Feldera's
pause/resume **snapshot** stall, which only applies to a cold replica reconstituting from the
durable source (§3.4, task 9). Here both replicas are warm, so that stall is correctly absent.
