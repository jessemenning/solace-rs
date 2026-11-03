#!/usr/bin/env bash
# run.sh — prove two warm Feldera replicas stay in lockstep (§12 task 5, Capability-2).
#
# Stands up two fresh Feldera containers, each ingesting the SAME keyed Solace stream via
# its own queue, and checks that their `rolling` (per-key COUNT) view stays identical over
# time and after drain — the warm-redundancy foundation HA failover rests on. Views are
# read over REST /query, so no solace_output changelog handling is needed.
#
# Requires the continuum-payments broker running (SMF localhost:55555, SEMP localhost:8089,
# docker network continuum-payments-feldera_default). Leaves the broker untouched; removes
# its own Feldera containers on exit (KEEP=1 to leave them up for inspection).
#
# Env: RATE (150) DURATION_S (25) KEYS (8) NETWORK IMAGE PORT_A PORT_B

set -euo pipefail
cd "$(dirname "$0")"
RIG_ROOT=../..                       # solace-rs repo root (for the load_gen binary)

NETWORK="${NETWORK:-continuum-payments-feldera_default}"
IMAGE="${IMAGE:-ghcr.io/jessemenning/feldera:feat-solace-connector}"
PORT_A="${PORT_A:-8091}"; PORT_B="${PORT_B:-8092}"
FQ_A=fq-a; FQ_B=fq-b
PN_A=lockstep-a; PN_B=lockstep-b
CA=lockstep-feldera-a; CB=lockstep-feldera-b

SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN=default
INPUT_TOPIC="${INPUT_TOPIC:-failover/in}"
RATE="${RATE:-150}"; DURATION_S="${DURATION_S:-25}"; KEYS="${KEYS:-8}"

cleanup() { [[ "${KEEP:-0}" == "1" ]] || { docker rm -f "$CA" "$CB" >/dev/null 2>&1 || true; }; }
trap cleanup EXIT

semp() { # method url body  (idempotent: 400 already-exists = ok)
  local code; code=$(curl -s -o /tmp/ls_semp.json -w "%{http_code}" -X "$1" --user "$SU:$SP" \
    -H "Content-Type: application/json" -d "$3" "$2")
  [[ "$code" == "200" ]] && return 0
  { [[ "$code" == "400" ]] && grep -qiE "already exists" /tmp/ls_semp.json; } && return 0
  echo "  SEMP $1 ${2##*/} -> $code" >&2; cat /tmp/ls_semp.json >&2; echo >&2; return 1
}

echo "=== 1. provision queues (both subscribe ${INPUT_TOPIC}/>) ==="
semp PATCH "$SEMP/msgVpns/$VPN/clientProfiles/default" \
  '{"allowGuaranteedMsgSendEnabled":true,"allowGuaranteedMsgReceiveEnabled":true,"allowGuaranteedEndpointCreateEnabled":true}'
for Q in "$FQ_A" "$FQ_B"; do
  semp POST "$SEMP/msgVpns/$VPN/queues" \
    "{\"queueName\":\"$Q\",\"accessType\":\"non-exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":500}"
  semp POST "$SEMP/msgVpns/$VPN/queues/$Q/subscriptions" "{\"subscriptionTopic\":\"${INPUT_TOPIC}/>\"}"
  echo "  queue $Q ready"
done

echo "=== 2. start two Feldera containers ==="
docker rm -f "$CA" "$CB" >/dev/null 2>&1 || true
docker run -d --name "$CA" --network "$NETWORK" -p "${PORT_A}:8080" "$IMAGE" >/dev/null
docker run -d --name "$CB" --network "$NETWORK" -p "${PORT_B}:8080" "$IMAGE" >/dev/null
for P in "$PORT_A" "$PORT_B"; do
  echo -n "  waiting for Feldera on $P "
  for _ in $(seq 1 60); do
    curl -sf "http://localhost:$P/healthz" >/dev/null 2>&1 && { echo "up"; break; }
    echo -n "."; sleep 2
  done
done

load_program() { # port pipeline queue
  local port="$1" pn="$2" fq="$3" fl="http://localhost:$1"
  sed "s/__FQ__/$fq/" program.sql > /tmp/ls_${pn}.sql
  jq -Rs --arg n "$pn" '{name:$n, program_code:., runtime_config:{workers:2}}' < /tmp/ls_${pn}.sql \
    | curl -s -X PUT -H "Content-Type: application/json" -d @- "$fl/v0/pipelines/$pn" >/dev/null
  echo -n "  $pn compiling "
  for _ in $(seq 1 90); do
    st=$(curl -s "$fl/v0/pipelines/$pn" | jq -r '.program_status' 2>/dev/null || echo "?")
    [[ "$st" == "Success" ]] && { echo "compiled"; break; }
    [[ "$st" == *Error* || "$st" == *Failed* ]] && { echo "FAILED ($st)"; curl -s "$fl/v0/pipelines/$pn" | jq '.program_error' >&2; exit 1; }
    echo -n "."; sleep 2
  done
  curl -s -X POST "$fl/v0/pipelines/$pn/start" >/dev/null
  echo -n "  $pn starting "
  for _ in $(seq 1 60); do
    st=$(curl -s "$fl/v0/pipelines/$pn" | jq -r '.deployment_status' 2>/dev/null || echo "?")
    [[ "$st" == "Running" ]] && { echo "running"; break; }
    echo -n "."; sleep 2
  done
}
echo "=== 3. load + start both pipelines ==="
load_program "$PORT_A" "$PN_A" "$FQ_A"
load_program "$PORT_B" "$PN_B" "$FQ_B"

query_view() { # port pipeline outfile
  curl -s -G "http://localhost:$1/v0/pipelines/$2/query" \
    --data-urlencode "sql=SELECT k, cnt FROM rolling ORDER BY k" \
    --data-urlencode "format=json" > "$3" 2>/dev/null || true
}

echo "=== 4. drive keyed load (${RATE} msg/s × ${DURATION_S}s, ${KEYS} keys) ==="
KEYED=1 SOLACE_BROKER_PASSWORD=default INPUT_TOPIC="$INPUT_TOPIC" RATE="$RATE" DURATION_S="$DURATION_S" KEYS="$KEYS" \
  "$RIG_ROOT/target/debug/examples/load_gen" >/tmp/ls_load.out 2>&1 &
LOAD_PID=$!

echo "=== 5. sample lockstep during load ==="
for i in 1 2 3 4; do
  sleep $(( DURATION_S / 5 ))
  query_view "$PORT_A" "$PN_A" /tmp/ls_a.json
  query_view "$PORT_B" "$PN_B" /tmp/ls_b.json
  echo "--- sample $i ---"; python3 -I compare.py /tmp/ls_a.json /tmp/ls_b.json
done

wait "$LOAD_PID" 2>/dev/null || true
echo "=== 6. final lockstep after drain ==="
sleep 4
query_view "$PORT_A" "$PN_A" /tmp/ls_a.json
query_view "$PORT_B" "$PN_B" /tmp/ls_b.json
python3 -I compare.py /tmp/ls_a.json /tmp/ls_b.json
echo
echo "(KEEP=1 to leave containers $CA/$CB up for inspection; default removes them.)"
