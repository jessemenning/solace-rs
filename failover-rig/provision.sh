#!/usr/bin/env bash
# provision.sh — prepare a local Solace broker for the two-replica failover rig (§12 task 5).
#
# Idempotent. Creates, on VPN `default`:
#   1. guaranteed messaging enabled on the `default` client-profile
#   2. an EXCLUSIVE queue `leader-q` (the leader lock + work queue)
#   3. a topic subscription `failover/in` on that queue (so load_gen's input is spooled)
#
# Targets a LOCAL software broker over plain-HTTP SEMP (defaults match the payments stack:
# SEMP on host 8089 → container 8080, admin/admin). Override via env if needed.
#
# Env overrides:
#   SEMP_BASE   (http://localhost:8089/SEMP/v2/config)
#   SEMP_USER   (admin)      SEMP_PASS (admin)
#   VPN         (default)    CLIENT_PROFILE (default)
#   LEADER_Q    (leader-q)   INPUT_TOPIC (failover/in)

set -euo pipefail

SEMP_BASE="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"
SEMP_USER="${SEMP_USER:-admin}"
SEMP_PASS="${SEMP_PASS:-admin}"
VPN="${VPN:-default}"
CLIENT_PROFILE="${CLIENT_PROFILE:-default}"
LEADER_Q="${LEADER_Q:-leader-q}"
INPUT_TOPIC="${INPUT_TOPIC:-failover/in}"

echo "==> SEMP   : ${SEMP_BASE}"
echo "==> VPN    : ${VPN}"
echo "==> Queue  : ${LEADER_Q} (exclusive)  <= topic '${INPUT_TOPIC}'"
echo

# POST/PATCH helper: treats HTTP 400 "already exists" as success (idempotent).
semp() {
  local method="$1" url="$2" body="$3"
  local code
  code=$(curl -s -o /tmp/failover_semp.json -w "%{http_code}" \
    -X "${method}" --user "${SEMP_USER}:${SEMP_PASS}" \
    -H "Content-Type: application/json" -d "${body}" "${url}")
  if [[ "${code}" == "200" ]]; then
    return 0
  fi
  local err
  err=$(jq -r '.meta.error.status // empty' /tmp/failover_semp.json 2>/dev/null || true)
  if [[ "${code}" == "400" ]] && grep -qiE "already exists|ALREADY_EXISTS" /tmp/failover_semp.json; then
    return 0
  fi
  echo "  ${method} ${url##*/} → HTTP ${code} (${err})" >&2
  cat /tmp/failover_semp.json >&2; echo >&2
  return 1
}

echo "--- 1. enable guaranteed messaging on client-profile '${CLIENT_PROFILE}' ---"
semp PATCH "${SEMP_BASE}/msgVpns/${VPN}/clientProfiles/${CLIENT_PROFILE}" '{
  "allowGuaranteedMsgSendEnabled": true,
  "allowGuaranteedMsgReceiveEnabled": true,
  "allowGuaranteedEndpointCreateEnabled": true
}'
echo "  ok"

echo "--- 2. create exclusive queue '${LEADER_Q}' ---"
semp POST "${SEMP_BASE}/msgVpns/${VPN}/queues" "{
  \"queueName\": \"${LEADER_Q}\",
  \"accessType\": \"exclusive\",
  \"ingressEnabled\": true,
  \"egressEnabled\": true,
  \"permission\": \"consume\",
  \"maxMsgSpoolUsage\": 500
}"
echo "  ok"

echo "--- 3. subscribe queue '${LEADER_Q}' to topic '${INPUT_TOPIC}' ---"
# subscriptionTopic path-encodes '/' as is in the body, not the URL.
semp POST "${SEMP_BASE}/msgVpns/${VPN}/queues/${LEADER_Q}/subscriptions" "{
  \"subscriptionTopic\": \"${INPUT_TOPIC}\"
}"
echo "  ok"

echo
echo "==> provisioned. Queue '${LEADER_Q}' is exclusive and attracts '${INPUT_TOPIC}'."
