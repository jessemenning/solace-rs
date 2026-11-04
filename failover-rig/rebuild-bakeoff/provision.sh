#!/usr/bin/env bash
# provision.sh — broker setup for the rebuild bake-off (shadow vs replay).
#
# Idempotent. On VPN `default` (SEMP admin/admin, default localhost:8089):
#   1. guaranteed messaging on the `default` client-profile
#   2. a VPN replay LOG (records guaranteed msgs for the replay path, §3.7)
#   3. bakeoff-src-q    — EXCLUSIVE (replay flows require exclusive), <= bakeoff/src/>
#   4. bakeoff-shadow-q — EXCLUSIVE, maxTtl = HORIZON_S, <= bakeoff/src/>  (shadow path, §3.6)
#
# Both queues subscribe to the same backlog topic, so one preload populates both; the replay
# log records the same stream for the replay path.
#
# Env: SEMP_BASE (http://localhost:8089/SEMP/v2/config), SEMP_USER/PASS (admin/admin),
#      VPN (default), SRC_TOPIC (bakeoff/src), HORIZON_S (3600), REPLAY_MB (500)

set -euo pipefail
SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN="${VPN:-default}"
SRC_TOPIC="${SRC_TOPIC:-bakeoff/src}"; HORIZON_S="${HORIZON_S:-3600}"; REPLAY_MB="${REPLAY_MB:-500}"
SRC_Q=bakeoff-src-q; SHADOW_Q=bakeoff-shadow-q; REPLAY_LOG=bakeoff-replay

semp() { # method url body  (idempotent: 400 already-exists = ok)
  local code; code=$(curl -s -o /tmp/bakeoff_semp.json -w "%{http_code}" -X "$1" --user "$SU:$SP" \
    -H "Content-Type: application/json" -d "$3" "$2")
  [[ "$code" == 200 ]] && return 0
  { [[ "$code" == 400 ]] && grep -qiE "already exists" /tmp/bakeoff_semp.json; } && return 0
  echo "  SEMP $1 ${2##*/} -> $code" >&2; cat /tmp/bakeoff_semp.json >&2; echo >&2; return 1
}
mk_queue() { # name
  semp POST "$SEMP/msgVpns/$VPN/queues" "{
    \"queueName\":\"$1\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,
    \"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":2000,
    \"respectTtlEnabled\":true,\"maxTtl\":$HORIZON_S}"
  semp POST "$SEMP/msgVpns/$VPN/queues/$1/subscriptions" "{\"subscriptionTopic\":\"$SRC_TOPIC/>\"}"
}

echo "==> VPN=$VPN  src_topic=$SRC_TOPIC/>  horizon=${HORIZON_S}s  replay_log=${REPLAY_MB}MB"
echo "--- 1. guaranteed messaging ---"
semp PATCH "$SEMP/msgVpns/$VPN/clientProfiles/default" \
  '{"allowGuaranteedMsgSendEnabled":true,"allowGuaranteedMsgReceiveEnabled":true,"allowGuaranteedEndpointCreateEnabled":true}'
echo "  ok"
echo "--- 2. replay log '$REPLAY_LOG' ---"
semp POST "$SEMP/msgVpns/$VPN/replayLogs" \
  "{\"replayLogName\":\"$REPLAY_LOG\",\"maxSpoolUsage\":$REPLAY_MB,\"ingressEnabled\":true,\"egressEnabled\":true}"
echo "  ok"
echo "--- 3. replay source queue '$SRC_Q' (exclusive) ---"; mk_queue "$SRC_Q"; echo "  ok"
echo "--- 4. shadow queue '$SHADOW_Q' (exclusive, maxTtl=${HORIZON_S}s) ---"; mk_queue "$SHADOW_Q"; echo "  ok"
echo "==> provisioned."
