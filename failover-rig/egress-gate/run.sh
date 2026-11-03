#!/usr/bin/env bash
# run.sh <graceful|ungraceful> — end-to-end Capability-2 failover (§12 task 5).
#
# Two warm Feldera replicas each publish their `rolling` view (solace_output). A per-replica
# egress gate holds the leader lock and relays ONLY the leader's output to the shared
# downstream. Kill the leader gate mid-load; the standby gate (whose replica is already warm)
# takes over and relays its already-correct output. Measures the downstream gap and proves
# there is NO per-key count reset across the handoff.
#
# Requires the continuum-payments broker (SMF localhost:55555, SEMP localhost:8089, network
# continuum-payments-feldera_default). Removes its own Feldera containers on exit (KEEP=1 to
# leave them). Env: RATE (120) DURATION_S (30) KEYS (8) KILL_AT_S (DURATION/2) MODE arg.

set -euo pipefail
MODE="${1:-graceful}"
case "$MODE" in graceful|ungraceful) ;; *) echo "usage: run.sh <graceful|ungraceful>" >&2; exit 2;; esac
cd "$(dirname "$0")"
ROOT=../..; BIN=$ROOT/target/debug/examples

NETWORK="${NETWORK:-continuum-payments-feldera_default}"
IMAGE="${IMAGE:-ghcr.io/jessemenning/feldera:feat-solace-connector}"
PORT_A=8091; PORT_B=8092; FQ_A=eg-fq-a; FQ_B=eg-fq-b; PN_A=egress-a; PN_B=egress-b
CA=egress-feldera-a; CB=egress-feldera-b; LEADER_Q=eg-leader-q
SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN=default
export SOLACE_BROKER_URL=tcp://localhost:55555 SOLACE_BROKER_PASSWORD=default LEADER_Q="$LEADER_Q"
RATE="${RATE:-120}"; DURATION_S="${DURATION_S:-30}"; KEYS="${KEYS:-8}"; KILL_AT_S="${KILL_AT_S:-$(( DURATION_S/2 ))}"
OUT_FILE=/tmp/egress-out.jsonl; A_OUT=/tmp/egress-gate-a.out; B_OUT=/tmp/egress-gate-b.out

PIDS=(); cleanup(){ for p in "${PIDS[@]:-}"; do kill -9 "$p" 2>/dev/null||true; done; [[ "${KEEP:-0}" == 1 ]]||docker rm -f "$CA" "$CB" >/dev/null 2>&1||true; }
trap cleanup EXIT
semp(){ local c; c=$(curl -s -o /tmp/eg_semp.json -w "%{http_code}" -X "$1" --user "$SU:$SP" -H "Content-Type: application/json" -d "$3" "$2"); [[ "$c" == 200 ]]&&return 0; { [[ "$c" == 400 ]]&&grep -qiE "already exists" /tmp/eg_semp.json; }&&return 0; echo "SEMP $1 ${2##*/} -> $c">&2; cat /tmp/eg_semp.json>&2; return 1; }

echo "=== 1. provision: input queues, exclusive leader lock ==="
semp PATCH "$SEMP/msgVpns/$VPN/clientProfiles/default" '{"allowGuaranteedMsgSendEnabled":true,"allowGuaranteedMsgReceiveEnabled":true,"allowGuaranteedEndpointCreateEnabled":true}'
for Q in "$FQ_A" "$FQ_B"; do
  semp POST "$SEMP/msgVpns/$VPN/queues" "{\"queueName\":\"$Q\",\"accessType\":\"non-exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":500}"
  semp POST "$SEMP/msgVpns/$VPN/queues/$Q/subscriptions" "{\"subscriptionTopic\":\"failover/in/>\"}"
done
semp POST "$SEMP/msgVpns/$VPN/queues" "{\"queueName\":\"$LEADER_Q\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":10}"
echo "  queues ready (fq a/b non-exclusive, $LEADER_Q exclusive lock)"

echo "=== 2. build examples + start two Feldera ==="
( cd "$ROOT" && cargo build -q --example egress_gate --example load_gen --example output_recorder )
docker rm -f "$CA" "$CB" >/dev/null 2>&1 || true
docker run -d --name "$CA" --network "$NETWORK" -p "$PORT_A:8080" "$IMAGE" >/dev/null
docker run -d --name "$CB" --network "$NETWORK" -p "$PORT_B:8080" "$IMAGE" >/dev/null
for P in "$PORT_A" "$PORT_B"; do echo -n "  feldera $P "; for _ in $(seq 1 60); do curl -sf "http://localhost:$P/healthz">/dev/null 2>&1&&{ echo up;break;}; echo -n .; sleep 2; done; done

load_pipe(){ local port=$1 pn=$2 fq=$3 fout=$4 fl="http://localhost:$1"
  sed -e "s/__FQ__/$fq/" -e "s#__FOUT__#$fout#" program.sql > /tmp/eg_$pn.sql
  jq -Rs --arg n "$pn" '{name:$n,program_code:.,runtime_config:{workers:2}}' < /tmp/eg_$pn.sql | curl -s -X PUT -H "Content-Type: application/json" -d @- "$fl/v0/pipelines/$pn">/dev/null
  echo -n "  $pn compiling "; for _ in $(seq 1 90); do st=$(curl -s "$fl/v0/pipelines/$pn"|jq -r .program_status 2>/dev/null||echo ?); [[ "$st" == Success ]]&&{ echo compiled;break;}; [[ "$st" == *Error* ]]&&{ echo "FAIL $st"; curl -s "$fl/v0/pipelines/$pn"|jq .program_error>&2; exit 1;}; echo -n .; sleep 2; done
  curl -s -X POST "$fl/v0/pipelines/$pn/start">/dev/null
  echo -n "  $pn starting "; for _ in $(seq 1 60); do [[ "$(curl -s "$fl/v0/pipelines/$pn"|jq -r .deployment_status 2>/dev/null)" == Running ]]&&{ echo running;break;}; echo -n .; sleep 2; done
}
echo "=== 3. load+start both pipelines (sinks → failover/fout/a|b) ==="
load_pipe "$PORT_A" "$PN_A" "$FQ_A" "failover/fout/a"
load_pipe "$PORT_B" "$PN_B" "$FQ_B" "failover/fout/b"

echo "=== 4. start recorder + two gates ==="
RUN_S=$(( DURATION_S + 12 )) OUT_FILE="$OUT_FILE" OUTPUT_TOPIC="failover/out/>" "$BIN/output_recorder" >/tmp/egress-rec.out 2>&1 &  REC=$!; PIDS+=("$REC")
sleep 1
GATE_IN="failover/fout/a/>" GATE_OUT="failover/out/a" "$BIN/egress_gate" A >"$A_OUT" 2>&1 &  GA=$!; PIDS+=("$GA")
GATE_IN="failover/fout/b/>" GATE_OUT="failover/out/b" "$BIN/egress_gate" B >"$B_OUT" 2>&1 &  GB=$!; PIDS+=("$GB")

LEADER=""; LPID=""; for _ in $(seq 1 20); do sleep 0.5
  a=$(grep -c "A ACTIVE" "$A_OUT" 2>/dev/null||true); a=${a:-0}; b=$(grep -c "B ACTIVE" "$B_OUT" 2>/dev/null||true); b=${b:-0}
  [[ "$a" -gt 0 && "$b" -eq 0 ]]&&{ LEADER=A; LPID=$GA; break;}; [[ "$b" -gt 0 && "$a" -eq 0 ]]&&{ LEADER=B; LPID=$GB; break;}
done
[[ -z "$LEADER" ]]&&{ echo "FAIL: no single leader gate">&2; exit 1; }
echo "  leader gate = $LEADER (pid $LPID)"

echo "=== 5. drive keyed load, kill leader @ ${KILL_AT_S}s ($MODE) ==="
KEYED=1 RATE="$RATE" DURATION_S="$DURATION_S" KEYS="$KEYS" INPUT_TOPIC=failover/in "$BIN/load_gen" >/tmp/egress-load.out 2>&1 &  LOAD=$!; PIDS+=("$LOAD")
sleep "$KILL_AT_S"
if [[ "$MODE" == graceful ]]; then echo ">>> SIGTERM gate $LEADER"; kill -TERM "$LPID" 2>/dev/null||true; else echo ">>> SIGKILL gate $LEADER"; kill -9 "$LPID" 2>/dev/null||true; fi
wait "$LOAD" 2>/dev/null||true; sleep 6

echo; echo "=== gate timelines ==="; grep -E "ACTIVE|INACTIVE|STOPPED" "$A_OUT" "$B_OUT" 2>/dev/null||true
echo; python3 -I analyze.py "$OUT_FILE"
echo; echo "(KEEP=1 leaves $CA/$CB up.)"
