#!/usr/bin/env bash
# configure-broker.sh
#
# Prepares a Solace Cloud broker for running async integration tests:
#   1. Enables guaranteed messaging on the client profile
#   2. Creates the durable test queue (idempotent — safe to rerun)
#
# Usage:
#   SEMP_USERNAME=<mgmt-user> SEMP_PASSWORD=<mgmt-pass> ./scripts/configure-broker.sh
#
# The messaging credentials (SOLACE_BROKER_*) are read from .env automatically.
# SEMP management credentials are separate and found in the Solace Cloud console
# under: Connect → SEMP REST API → Management Username / Management Password.
#
# Optional overrides:
#   SOLACE_TEST_QUEUE   queue name (default: rust-test-queue)
#   SEMP_PORT           SEMP management port (default: 943)

set -euo pipefail

# ---------------------------------------------------------------------------
# Load messaging config from .env
# ---------------------------------------------------------------------------
if [[ -f .env ]]; then
  set -a; source .env; set +a
fi

# ---------------------------------------------------------------------------
# Validate required env vars
# ---------------------------------------------------------------------------
: "${SOLACE_BROKER_URL:?SOLACE_BROKER_URL not set (expected in .env)}"
: "${SOLACE_BROKER_VPN:?SOLACE_BROKER_VPN not set (expected in .env)}"
: "${SOLACE_BROKER_USERNAME:?SOLACE_BROKER_USERNAME not set (expected in .env)}"
: "${SEMP_USERNAME:?Set SEMP_USERNAME to the broker management username (from Solace Cloud console → Connect → SEMP REST API)}"
: "${SEMP_PASSWORD:?Set SEMP_PASSWORD to the broker management password}"

QUEUE="${SOLACE_TEST_QUEUE:-rust-test-queue}"
SEMP_PORT="${SEMP_PORT:-943}"

# ---------------------------------------------------------------------------
# Derive SEMP base URL from messaging URL
# wss://host:port  →  https://host:943
# ---------------------------------------------------------------------------
BROKER_HOST=$(printf '%s' "${SOLACE_BROKER_URL}" | sed -E 's|^[a-z+]+://||; s|:[0-9]+/?.*$||')
SEMP_BASE="https://${BROKER_HOST}:${SEMP_PORT}/SEMP/v2/config"
VPN="${SOLACE_BROKER_VPN}"
CLIENT_USERNAME="${SOLACE_BROKER_USERNAME}"

echo "==> SEMP base : ${SEMP_BASE}"
echo "==> VPN       : ${VPN}"
echo "==> Client    : ${CLIENT_USERNAME}"
echo "==> Queue     : ${QUEUE}"
echo

# ---------------------------------------------------------------------------
# Helper: issue a SEMP request; print status + body on error
# ---------------------------------------------------------------------------
semp_req() {
  local method="$1"; shift
  local url="$1"; shift

  local http_code
  local body
  body=$(curl -s -o /tmp/semp_resp.json -w "%{http_code}" \
              -X "${method}" \
              --user "${SEMP_USERNAME}:${SEMP_PASSWORD}" \
              -H "Content-Type: application/json" \
              "$@" \
              "${url}")
  http_code="${body}"

  if [[ "${http_code}" -ge 400 ]]; then
    echo "  HTTP ${http_code}" >&2
    cat /tmp/semp_resp.json >&2
    echo >&2
    return 1
  fi

  echo "${http_code}"
}

# ---------------------------------------------------------------------------
# 1. Discover client profile for this username
# ---------------------------------------------------------------------------
echo "--- Step 1: find client profile for '${CLIENT_USERNAME}' ---"
curl -sf \
     --user "${SEMP_USERNAME}:${SEMP_PASSWORD}" \
     "${SEMP_BASE}/msgVpns/${VPN}/clientUsernames/${CLIENT_USERNAME}" \
  > /tmp/semp_resp.json

PROFILE=$(jq -r '.data.clientProfileName' /tmp/semp_resp.json)
echo "  client profile: ${PROFILE}"
echo

# ---------------------------------------------------------------------------
# 2. Patch client profile — enable guaranteed messaging
# ---------------------------------------------------------------------------
echo "--- Step 2: enable guaranteed messaging on profile '${PROFILE}' ---"
code=$(semp_req PATCH \
  "${SEMP_BASE}/msgVpns/${VPN}/clientProfiles/${PROFILE}" \
  -d '{
    "allowGuaranteedMsgSendEnabled": true,
    "allowGuaranteedMsgReceiveEnabled": true,
    "allowGuaranteedEndpointCreateEnabled": true,
    "allowTransactedSessionsEnabled": true
  }')
echo "  PATCH client profile → HTTP ${code}"
echo

# ---------------------------------------------------------------------------
# 3. Create the test queue (idempotent — ignore 400 ALREADY_EXISTS)
# ---------------------------------------------------------------------------
echo "--- Step 3: provision queue '${QUEUE}' ---"
http_code=$(curl -s -o /tmp/semp_resp.json -w "%{http_code}" \
  -X POST \
  --user "${SEMP_USERNAME}:${SEMP_PASSWORD}" \
  -H "Content-Type: application/json" \
  "${SEMP_BASE}/msgVpns/${VPN}/queues" \
  -d "{
    \"queueName\": \"${QUEUE}\",
    \"accessType\": \"exclusive\",
    \"ingressEnabled\": true,
    \"egressEnabled\": true,
    \"permission\": \"consume\",
    \"maxMsgSpoolUsage\": 100
  }")

if [[ "${http_code}" == "200" ]]; then
  echo "  Queue created: ${QUEUE}"
elif [[ "${http_code}" == "400" ]]; then
  error_id=$(jq -r '.meta.error.code // empty' /tmp/semp_resp.json 2>/dev/null)
  if [[ "${error_id}" == "89" ]] || grep -qi "already exists" /tmp/semp_resp.json; then
    echo "  Queue already exists — skipping"
  else
    echo "  POST queue → HTTP ${http_code} (unexpected)" >&2
    cat /tmp/semp_resp.json >&2
    exit 1
  fi
else
  echo "  POST queue → unexpected HTTP ${http_code}" >&2
  cat /tmp/semp_resp.json >&2
  exit 1
fi
echo

echo "==> Done. Run the tests with:"
echo "    set -a && source .env && set +a && \\"
echo "    cargo test --features async --test async_integration_test -- --include-ignored"
