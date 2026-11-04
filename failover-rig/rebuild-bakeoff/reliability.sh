#!/usr/bin/env bash
# reliability.sh — the loud-failure matrix. The whole OSS-rebuild story rests on a gap failing
# LOUD (visible, refused, or detectably short), never a silent wrong rebuild. Each case sets up
# a failure and asserts a non-silent signal.
#
#   1. replay, non-exclusive endpoint  -> client-initiated replay REFUSED (build error, subcode 136)
#   2. replay, log trimmed inside need -> replay delivers FEWER than published (short, detectable)
#   3. shadow, TTL expired inside need -> browse drains FEWER than published (short, detectable)
#
# Env: SOLACE_BROKER_PASSWORD (default), SEMP_*/VPN as usual.

set -euo pipefail
cd "$(dirname "$0")"; ROOT=../..; BIN=$ROOT/target/debug/examples
SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; MON="${SEMP%/config}/monitor"
SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN="${VPN:-default}"
SRC_TOPIC="${SRC_TOPIC:-bakeoff/src}"; SRC_Q=bakeoff-src-q; SHADOW_Q=bakeoff-shadow-q; REPLAY_LOG=bakeoff-replay
export SOLACE_BROKER_URL="${SOLACE_BROKER_URL:-tcp://localhost:55555}" SOLACE_BROKER_PASSWORD="${SOLACE_BROKER_PASSWORD:-default}"

d() { curl -s -o /dev/null -X DELETE --user "$SU:$SP" "$SEMP/msgVpns/$VPN/$1" 2>/dev/null || true; }
post() { curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" "$SEMP/msgVpns/$VPN/$1" -d "$2"; }
sub() { post "queues/$1/subscriptions" "{\"subscriptionTopic\":\"$SRC_TOPIC/>\"}"; }
reset_all() { d "queues/$SRC_Q"; d "queues/$SHADOW_Q"; d "replayLogs/$REPLAY_LOG"; sleep 1; }
preload() { KEYED=1 NODELAY=1 INPUT_TOPIC="$SRC_TOPIC" RATE="$1" DURATION_S=1 KEYS=8 "$BIN/load_gen" 2>/dev/null >/dev/null; sleep 1; }
result() { "$@" 2>/dev/null | grep -E "RESULT"; }
pass() { echo "  PASS (loud): $1"; }; fail() { echo "  FAIL (SILENT!): $1"; }

echo "===================================================================="
echo " Loud-failure matrix — a rebuild gap must never be silent"
echo "===================================================================="

echo; echo "### 1. Replay on a NON-EXCLUSIVE endpoint -> must be REFUSED"
reset_all
post "replayLogs" "{\"replayLogName\":\"$REPLAY_LOG\",\"maxSpoolUsage\":200,\"ingressEnabled\":true,\"egressEnabled\":true}"
post "queues" "{\"queueName\":\"$SRC_Q\",\"accessType\":\"non-exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":500}"; sub "$SRC_Q"
preload 2000
R=$(SRC_QUEUE="$SRC_Q" REPLAY_START=BEGINNING EMPTY_TIMEOUT_MS=8000 result "$BIN/rebuild_refeeder" replay)
echo "  $R"
if echo "$R" | grep -qE "FLOW_BUILD_ERROR|failure_event=true|drained=0"; then pass "replay refused on non-exclusive endpoint"; else fail "non-exclusive replay was NOT refused"; fi

echo; echo "### 2. Replay with the log TRIMMED (log too small for the backlog) -> short/again loud"
reset_all
post "replayLogs" "{\"replayLogName\":\"$REPLAY_LOG\",\"maxSpoolUsage\":1,\"ingressEnabled\":true,\"egressEnabled\":true}"   # 1 MB — trims under load
post "queues" "{\"queueName\":\"$SRC_Q\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":2000}"; sub "$SRC_Q"
preload 60000
# empty the queue so delivery is from the (trimmed) log only
d "queues/$SRC_Q"; post "queues" "{\"queueName\":\"$SRC_Q\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":2000}"; sub "$SRC_Q"
R=$(SRC_QUEUE="$SRC_Q" REPLAY_START=BEGINNING EMPTY_TIMEOUT_MS=8000 result "$BIN/rebuild_refeeder" replay)
echo "  $R  (published 60000)"
drained=$(echo "$R" | grep -oE "drained=[0-9]+" | cut -d= -f2)
if echo "$R" | grep -qE "failure_event=true" || [ "${drained:-0}" -lt 60000 ]; then pass "trimmed replay is detectable (drained=$drained < 60000, or failure event)"; else fail "trimmed replay silently looked complete"; fi

echo; echo "### 3. Shadow with TTL EXPIRED (maxTtl short) -> browse drains short"
reset_all
post "queues" "{\"queueName\":\"$SHADOW_Q\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":500,\"respectTtlEnabled\":true,\"maxTtl\":3}"; sub "$SHADOW_Q"
preload 3000
echo "  waiting 6s for the 3s TTL to expire the backlog ..."; sleep 6
R=$(SRC_QUEUE="$SHADOW_Q" result "$BIN/rebuild_refeeder" shadow)
spooled=$(curl -s --user "$SU:$SP" "$MON/msgVpns/$VPN/queues/$SHADOW_Q" | python3 -I -c "import json,sys;print(json.load(sys.stdin).get('data',{}).get('spooledMsgCount','?'))" 2>/dev/null || echo "?")
expired=$(curl -s --user "$SU:$SP" "$MON/msgVpns/$VPN/queues/$SHADOW_Q" | python3 -I -c "import json,sys;print(json.load(sys.stdin).get('data',{}).get('maxTtlExpiredDiscardedMsgCount','?'))" 2>/dev/null || echo "?")
echo "  $R  (published 3000, ttl-expired discarded=$expired)"
drained=$(echo "$R" | grep -oE "drained=[0-9]+" | cut -d= -f2)
if [ "${drained:-0}" -lt 3000 ]; then pass "shadow drain is short after TTL expiry (drained=$drained < 3000, expired=$expired)"; else fail "shadow drain looked complete despite TTL expiry"; fi

echo; echo "===================================================================="
echo " Done. A loud failure = refused / failure_event=true / drained < published."
echo "===================================================================="
