#!/usr/bin/env bash
# ingress-speed.sh — the STEADY-STATE cost of each rebuild approach.
#
# Rebuild speed is the recovery cost; this is the price each approach charges during NORMAL
# ingress. Publishes the same backlog as fast as possible (Persistent) under three configs and
# reports achieved publish throughput:
#   baseline  — one plain durable queue (no replay log, no shadow)
#   replay    — + VPN replay LOG on (every guaranteed msg also written to the log spool)
#   shadow    — + a 2nd durable queue on the same topic (every msg spooled TWICE)
#
# Env: INGRESS_N (50000), KEYS (16), SOLACE_BROKER_PASSWORD (default), SEMP_*/VPN as usual.

set -euo pipefail
cd "$(dirname "$0")"; ROOT=../..; BIN=$ROOT/target/debug/examples
SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN="${VPN:-default}"
SRC_TOPIC="${SRC_TOPIC:-bakeoff/src}"; SRC_Q=bakeoff-src-q; SHADOW_Q=bakeoff-shadow-q; REPLAY_LOG=bakeoff-replay
N="${INGRESS_N:-50000}"; KEYS="${KEYS:-16}"
export SOLACE_BROKER_URL="${SOLACE_BROKER_URL:-tcp://localhost:55555}" SOLACE_BROKER_PASSWORD="${SOLACE_BROKER_PASSWORD:-default}"

ROUNDS="${ROUNDS:-4}"   # measured rounds per config; results interleaved to cancel drift
d() { curl -s -o /dev/null -X DELETE --user "$SU:$SP" "$SEMP/msgVpns/$VPN/$1" 2>/dev/null || true; }
mkq() { curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" "$SEMP/msgVpns/$VPN/queues" \
  -d "{\"queueName\":\"$1\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":4000}"
  curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" "$SEMP/msgVpns/$VPN/queues/$1/subscriptions" -d "{\"subscriptionTopic\":\"$SRC_TOPIC/>\"}"; }
mklog() { curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" "$SEMP/msgVpns/$VPN/replayLogs" \
  -d "{\"replayLogName\":\"$REPLAY_LOG\",\"maxSpoolUsage\":2000,\"ingressEnabled\":true,\"egressEnabled\":true}"; }
reset_all() { d "queues/$SRC_Q"; d "queues/$SHADOW_Q"; d "replayLogs/$REPLAY_LOG"; sleep 1; }
# Recreate exactly the config's endpoints (clean queue each publish), then publish N.
setup() { reset_all; case "$1" in
    baseline) mkq "$SRC_Q";;
    replay)   mklog; mkq "$SRC_Q";;
    shadow)   mkq "$SRC_Q"; mkq "$SHADOW_Q";;
  esac; }
publish() { KEYED=1 NODELAY=1 INPUT_TOPIC="$SRC_TOPIC" RATE="$N" DURATION_S=1 KEYS="$KEYS" "$BIN/load_gen" 2>/dev/null \
  | grep -oE "throughput_msg_s=[0-9]+" | cut -d= -f2; }

curl -s -o /dev/null -X PATCH --user "$SU:$SP" -H "Content-Type: application/json" "$SEMP/msgVpns/$VPN/clientProfiles/default" \
  -d '{"allowGuaranteedMsgSendEnabled":true,"allowGuaranteedMsgReceiveEnabled":true,"allowGuaranteedEndpointCreateEnabled":true}'

echo "=== ingress throughput, $N msgs Persistent, $KEYS keys, $ROUNDS rounds (median) ==="
echo "--- warmup (discarded) ---"; setup baseline; publish >/dev/null; echo "  done"
declare -A R
for cfg in baseline replay shadow; do R[$cfg]=""; done
for ((i=1;i<=ROUNDS;i++)); do
  line="round $i:"
  for cfg in baseline replay shadow; do
    setup "$cfg"; t=$(publish); R[$cfg]="${R[$cfg]} $t"; line="$line  $cfg=$t"
  done
  echo "  $line"
done

echo
echo "=== SUMMARY (median achieved publish msg/s) ==="
BASE="${R[baseline]}" REP="${R[replay]}" SH="${R[shadow]}" python3 -I -c "
import os, statistics as st
R={k:[int(x) for x in os.environ[v].split()] for k,v in (('baseline','BASE'),('replay','REP'),('shadow','SH'))}
b=st.median(R['baseline'])
for cfg in ('baseline','replay','shadow'):
    m=st.median(R[cfg])
    print(f'  {cfg:8}: {m:>7.0f} msg/s  ({m/b:.2f}x baseline, {100*(m/b-1):+.0f}%)')
"
