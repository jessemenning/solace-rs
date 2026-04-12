#!/usr/bin/env bash
# teardown-broker.sh
#
# Reverses what configure-broker.sh created:
#   1. Deletes the test queue (rust-test-queue by default)
#   2. Disables guaranteed messaging on the client profile
#
# Usage:
#   SEMP_USERNAME=<mgmt-user> SEMP_PASSWORD=<mgmt-pass> ./scripts/teardown-broker.sh
#
# Options:
#   --keep-profile   Skip reverting the client profile (just delete the queue)

set -euo pipefail

KEEP_PROFILE=false
for arg in "$@"; do
  case "${arg}" in
    --keep-profile) KEEP_PROFILE=true ;;
    *) echo "Unknown argument: ${arg}"; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Load .env
# ---------------------------------------------------------------------------
if [[ -f .env ]]; then
  set -a; source .env; set +a
fi

: "${SOLACE_BROKER_URL:?SOLACE_BROKER_URL not set}"
: "${SOLACE_BROKER_VPN:?SOLACE_BROKER_VPN not set}"
: "${SOLACE_BROKER_USERNAME:?SOLACE_BROKER_USERNAME not set}"
: "${SEMP_USERNAME:?Set SEMP_USERNAME (from Solace Cloud console → Connect → SEMP REST API)}"
: "${SEMP_PASSWORD:?Set SEMP_PASSWORD}"

QUEUE="${SOLACE_TEST_QUEUE:-rust-test-queue}"
SEMP_PORT="${SEMP_PORT:-943}"

BROKER_HOST=$(printf '%s' "${SOLACE_BROKER_URL}" | sed -E 's|^[a-z+]+://||; s|:[0-9]+/?.*$||')
SEMP_BASE="https://${BROKER_HOST}:${SEMP_PORT}/SEMP/v2/config"
VPN="${SOLACE_BROKER_VPN}"
CLIENT_USERNAME="${SOLACE_BROKER_USERNAME}"

echo "==> SEMP base : ${SEMP_BASE}"
echo "==> VPN       : ${VPN}"
echo "==> Queue     : ${QUEUE}"
echo

# ---------------------------------------------------------------------------
# Helper
# ---------------------------------------------------------------------------
semp_req() {
  local method="$1"; shift
  local url="$1"; shift

  local http_code
  http_code=$(curl -s -o /tmp/semp_resp.json -w "%{http_code}" \
                   -X "${method}" \
                   --user "${SEMP_USERNAME}:${SEMP_PASSWORD}" \
                   -H "Content-Type: application/json" \
                   "$@" \
                   "${url}")

  echo "${http_code}"
}

# ---------------------------------------------------------------------------
# 1. Delete test queue
# ---------------------------------------------------------------------------
echo "--- Step 1: delete queue '${QUEUE}' ---"
code=$(semp_req DELETE "${SEMP_BASE}/msgVpns/${VPN}/queues/${QUEUE}")

if [[ "${code}" == "200" ]]; then
  echo "  Queue deleted."
elif [[ "${code}" == "400" ]] || [[ "${code}" == "404" ]]; then
  not_found=$(jq -r '.meta.error.code // empty' /tmp/semp_resp.json 2>/dev/null)
  if [[ "${not_found}" == "81" ]] || grep -qi "not found\|does not exist" /tmp/semp_resp.json 2>/dev/null; then
    echo "  Queue not found — nothing to delete."
  else
    echo "  DELETE queue → HTTP ${code}" >&2
    cat /tmp/semp_resp.json >&2
    exit 1
  fi
else
  echo "  DELETE queue → unexpected HTTP ${code}" >&2
  cat /tmp/semp_resp.json >&2
  exit 1
fi
echo

# ---------------------------------------------------------------------------
# 2. Revert client profile (unless --keep-profile)
# ---------------------------------------------------------------------------
if [[ "${KEEP_PROFILE}" == true ]]; then
  echo "--- Step 2: skipping client profile revert (--keep-profile) ---"
else
  echo "--- Step 2: revert guaranteed messaging on client profile ---"

  curl -sf \
       --user "${SEMP_USERNAME}:${SEMP_PASSWORD}" \
       "${SEMP_BASE}/msgVpns/${VPN}/clientUsernames/${CLIENT_USERNAME}" \
    > /tmp/semp_resp.json

  PROFILE=$(jq -r '.data.clientProfileName' /tmp/semp_resp.json)
  echo "  Client profile: ${PROFILE}"

  code=$(semp_req PATCH \
    "${SEMP_BASE}/msgVpns/${VPN}/clientProfiles/${PROFILE}" \
    -d '{
      "allowGuaranteedMsgSendEnabled": false,
      "allowGuaranteedMsgReceiveEnabled": false,
      "allowGuaranteedEndpointCreateEnabled": false,
      "allowTransactedSessionsEnabled": false
    }')
  echo "  PATCH client profile → HTTP ${code}"
fi

echo
echo "==> Teardown complete."
