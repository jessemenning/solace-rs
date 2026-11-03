#!/usr/bin/env bash
# run.sh <replay|shadow> — preload a backlog, then rebuild-drain it via the chosen source
# and print the RESULT line (drained count, span, throughput, order). Measures the SOURCE
# (broker replay-log re-delivery vs non-destructive browse drain); Feldera end-to-end
# catch-up is layered by the caller.
#
# Env: BACKLOG (messages, 20000), KEYS (16), SOLACE_BROKER_PASSWORD (default),
#      SEMP_* / VPN as in provision.sh.

set -euo pipefail
MODE="${1:-replay}"; case "$MODE" in replay|shadow) ;; *) echo "usage: run.sh <replay|shadow>">&2; exit 2;; esac
cd "$(dirname "$0")"; ROOT=../..; BIN=$ROOT/target/debug/examples
SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; MON="${SEMP%/config}/monitor"
SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN="${VPN:-default}"
SRC_TOPIC="${SRC_TOPIC:-bakeoff/src}"; SRC_Q=bakeoff-src-q; SHADOW_Q=bakeoff-shadow-q
BACKLOG="${BACKLOG:-20000}"; KEYS="${KEYS:-16}"
export SOLACE_BROKER_URL="${SOLACE_BROKER_URL:-tcp://localhost:55555}" SOLACE_BROKER_PASSWORD="${SOLACE_BROKER_PASSWORD:-default}"

qfield() { curl -s --user "$SU:$SP" "$MON/msgVpns/$VPN/queues/$1" 2>/dev/null \
  | python3 -I -c "import json,sys;print(json.load(sys.stdin).get('data',{}).get('$2','?'))" 2>/dev/null || echo "?"; }
qdepth()   { qfield "$1" spooledMsgCount; }
qdeleted() { qfield "$1" deletedMsgCount; }

echo "=== reset (clean slate) ==="
for q in "$SRC_Q" "$SHADOW_Q"; do curl -s -o /dev/null -X DELETE --user "$SU:$SP" "$SEMP/msgVpns/$VPN/queues/$q"; done
curl -s -o /dev/null -X DELETE --user "$SU:$SP" "$SEMP/msgVpns/$VPN/replayLogs/bakeoff-replay"
echo "  cleared queues + replay log"
echo "=== provision ==="; bash provision.sh >/dev/null && echo "  ok"

# Preload the backlog once (populates both queues + the replay log). High rate → self-throttles
# to the broker's persistent throughput; load_gen publishes exactly RATE*DURATION_S msgs.
RATE=$(( BACKLOG / 5 )); [ "$RATE" -lt 1 ] && RATE=$BACKLOG
echo "=== preload $BACKLOG msgs ($KEYS keys) to $SRC_TOPIC/> ==="
KEYED=1 INPUT_TOPIC="$SRC_TOPIC" RATE="$RATE" DURATION_S=5 KEYS="$KEYS" "$BIN/load_gen" 2>&1 | tail -1
sleep 2
echo "  src-q depth=$(qdepth $SRC_Q)  shadow-q depth=$(qdepth $SHADOW_Q)"

if [ "$MODE" = replay ]; then
  # Empty the replay source queue so the replay flow delivers purely from the LOG.
  echo "=== empty $SRC_Q (replay delivers from the log, not live spool) ==="
  curl -s -o /dev/null -X DELETE --user "$SU:$SP" "$SEMP/msgVpns/$VPN/queues/$SRC_Q"
  curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" \
    "$SEMP/msgVpns/$VPN/queues" -d "{\"queueName\":\"$SRC_Q\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":2000}"
  curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" \
    "$SEMP/msgVpns/$VPN/queues/$SRC_Q/subscriptions" -d "{\"subscriptionTopic\":\"$SRC_TOPIC/>\"}"
  echo "  src-q depth after recreate=$(qdepth $SRC_Q)"
  echo "=== REPLAY drain ==="
  SRC_QUEUE="$SRC_Q" REPLAY_START=BEGINNING "$BIN/rebuild_refeeder" replay 2>&1 | grep -E "RESULT|drained"
else
  echo "=== SHADOW browse-drain (non-destructive) ==="
  SRC_QUEUE="$SHADOW_Q" "$BIN/rebuild_refeeder" shadow 2>&1 | grep -E "RESULT|drained"
  spooled=$(qdepth $SHADOW_Q); deleted=$(qdeleted $SHADOW_Q)
  echo "  shadow-q spooled=$spooled deleted=$deleted $([ "$deleted" = "0" ] && echo '(NON-DESTRUCTIVE ✓ — messages still on queue)' || echo '(DESTRUCTIVE ✗)')"
fi
