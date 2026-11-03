# Rebuild bake-off: shadow queue vs Message Replay

Characterizes the two OSS ungraceful-restart rebuild sources (`OSS-HA-DR-DESIGN.md` §3.6
shadow queue, §3.7 Message Replay) head-to-head on **rebuild performance, steady-state
ingress cost, reliability, and elegance**, and makes the bake-off runnable as a demo.

Both re-read the same durable backlog to rebuild Feldera state after a crash. The difference:
- **Replay** (§3.7): the broker's VPN **replay log** re-delivers the (already-acked) history
  to an exclusive flow via `replay_start_location`. One flow, replay → live.
- **Shadow** (§3.6): a **second durable queue** (`max-ttl = horizon`) holds the backlog; a
  **non-destructive browser flow** reads it. Independent of the replay log (any broker/edition).

## Pieces

- `../../examples/rebuild_refeeder.rs` — drains a source and reports throughput/order.
  `replay` mode: exclusive flow + `replay_start_location`, client-ack to advance.
  `shadow` mode: `browser(true)` flow, Auto ack, **`flow.start()` to reset the browse window**
  (see "Browser semantics" below), dedup by seq.
- `provision.sh` — VPN replay log + exclusive `bakeoff-src-q` + shadow `bakeoff-shadow-q`
  (maxTtl=horizon), both subscribed to `bakeoff/src/>`.
- `run.sh <replay|shadow>` — clean-slate, preload a backlog, drain via the chosen source,
  report. Replay empties the queue first (delivery from the log only); shadow checks
  `deletedMsgCount==0` (non-destructive).
- `ingress-speed.sh` — publish throughput (max-speed, Persistent) under baseline / replay-log /
  shadow-double-spool, warmup + interleaved medians.

Run: `BACKLOG=50000 SOLACE_BROKER_PASSWORD=default bash run.sh replay` (or `shadow`);
`INGRESS_N=30000 bash ingress-speed.sh`.

## Measured (local Standard broker, 16 keys)

**Rebuild throughput** (drain the backlog):

| Source | 50k backlog | Full drain | Order | Non-destructive |
|--------|-------------|-----------|-------|-----------------|
| **Replay** | **~65,000 msg/s** | 50000/50000 ✓ | in-order ✓ | n/a (reads log) |
| **Shadow** | **~1,600 msg/s** | 50000/50000 ✓ | in-order ✓ | ✓ (deleted=0) |

Replay is **~40× faster** to rebuild (and the gap widens at smaller `RESET_MS`; shadow is
window-reset-bound — one ~255-message window per `flow.start()` cycle).

**Steady-state ingress** (publish throughput, warmup-corrected medians):

| Config | msg/s | vs baseline |
|--------|-------|-------------|
| baseline (plain queue) | ~11,250 | 1.00× |
| replay log on | ~11,330 | +1% |
| shadow (double-spool) | ~11,220 | −0% |

**Neither approach measurably taxes ingress** at ~11k msg/s — the extra replay-log write and
the shadow's double-spool are absorbed by the broker without backpressuring the publisher.
(First-run numbers that showed a 35–70% "penalty" were a warm-up artifact; always discard a
warmup and take medians — `ingress-speed.sh` does.)

## Browser semantics (the key shadow finding)

The shadow browse-drain first stalled at ~one window (~255). This is **not** a solace-rs bug —
it is inherent Solace browser behavior: on a browser flow `sendAck` **deletes** the message, it
does not advance a cursor; non-destructive progress comes only from calling
`solClient_flow_start()` to **reset the browse window** (the window does not auto-reopen). The
fix was in *usage*: `flow.start()` to reset, never `ack`. Consequence: a correct non-destructive
full drain is a reset-poll loop with a **stall heuristic** — "have I browsed everything, or has
the window just not refilled yet?" is genuinely ambiguous, so the loop risks stopping early at
low `RESET_MS`. Replay has none of this: the broker drives a clean replay→live handoff.

## Verdict

| Axis | Winner | Why |
|------|--------|-----|
| Rebuild performance | **Replay** | ~40× faster; broker-paced |
| Ingress cost | tie | both ~0 tax at tested rate |
| Reliability | **Replay** | clean broker-driven end; shadow's reset-poll stall heuristic can stop early |
| Elegance | **Replay** | one flow + `replay_start_location`; shadow = 2nd queue + TTL + browser + reset loop + dedup + heuristic |
| Portability | **Shadow** | no replay log required (any broker/edition) |

**Default: Replay** on any broker with a replay log (Standard and up — proven available here).
**Shadow only as a portability fallback** where the replay log is unavailable, accepting ~40×
slower rebuild and the reset-poll fragility.

## Reliability loud-failure matrix (`reliability.sh`, measured)

A rebuild gap must fail LOUD (refused / error / detectably short), never a silent wrong
rebuild. All three cases surface non-silently:

| Case | Setup | Loud signal |
|------|-------|-------------|
| Non-exclusive replay | client-initiated replay on a non-exclusive queue | **refused at bind, subcode 135** ("Client Initiated Replay Not Allowed on Non-Exclusive Queue") — note: **135**, not 136 |
| Trimmed replay log | 1 MB log, 60k backlog, replay from BEGINNING | drained **13,355 / 60,000, no broker error** — short but *silent at the broker*; loud only if the sidecar checks count vs watermark |
| Shadow TTL expired | `maxTtl=3s`, publish, wait 6s, browse | drains **empty/short** (0 of 3000), detectable by count |

**Key finding:** replay from **BEGINNING** on a trimmed log does **not** raise 139/140 — it
silently delivers what remains. The loud-failure subcodes (139 `START_TIME_NOT_AVAILABLE`,
140 `MESSAGE_UNAVAILABLE`) fire only for a trimmed **RGMID/timestamp** start. So the rebuild
must **assert it reached its expected watermark**; "no error" ≠ "no gap." The re-feeder reports
`drained=` and `failure_event=` for exactly this check.

## Remaining (not in this cut)

- Feldera end-to-end catch-up (re-feed the drained stream into a pipeline; this rig measures
  the source layer, the dominant differentiator).
- A presentation UI (this rig's tables are the terminal demo).
