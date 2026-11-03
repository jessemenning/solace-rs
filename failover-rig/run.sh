#!/usr/bin/env bash
# run.sh <graceful|ungraceful> — one forced-failover run of the two-replica rig (§12 task 5).
#
# Orchestrates: provision broker → start recorder → start replica A + B (exclusive leader-q,
# only the Active one publishes) → start load_gen → mid-load drop the leader → let the
# standby take over → analyze.
#
#   graceful    SIGTERM the leader; its handler drops the flow (clean unbind).
#   ungraceful  SIGKILL the leader; the broker detects the lost flow and redelivers unacked.
#
# Env overrides:
#   RATE (200 msg/s)  DURATION_S (20)  KEYS (8)  KILL_AT_S (DURATION_S/2)
#   TAIL_S (8, recorder slack after load ends)  OUT_FILE (/tmp/failover-out.jsonl)
#   SOLACE_BROKER_URL (tcp://localhost:55555)  LEADER_Q (leader-q)
#   INPUT_TOPIC (failover/in)  OUTPUT_TOPIC (failover/out)

set -euo pipefail
MODE="${1:-graceful}"
case "${MODE}" in graceful|ungraceful) ;; *) echo "usage: run.sh <graceful|ungraceful>" >&2; exit 2;; esac

cd "$(dirname "$0")/.."                          # solace-rs repo root
RIG="failover-rig"

export SOLACE_BROKER_URL="${SOLACE_BROKER_URL:-tcp://localhost:55555}"
export SOLACE_BROKER_VPN="${SOLACE_BROKER_VPN:-default}"
export SOLACE_BROKER_USERNAME="${SOLACE_BROKER_USERNAME:-default}"
export SOLACE_BROKER_PASSWORD="${SOLACE_BROKER_PASSWORD:-}"
export LEADER_Q="${LEADER_Q:-leader-q}"
export INPUT_TOPIC="${INPUT_TOPIC:-failover/in}"
export OUTPUT_TOPIC="${OUTPUT_TOPIC:-failover/out}"

RATE="${RATE:-200}"
DURATION_S="${DURATION_S:-20}"
KEYS="${KEYS:-8}"
KILL_AT_S="${KILL_AT_S:-$(( DURATION_S / 2 ))}"
TAIL_S="${TAIL_S:-8}"
OUT_FILE="${OUT_FILE:-/tmp/failover-out.jsonl}"

A_OUT=/tmp/failover-A.out
B_OUT=/tmp/failover-B.out
BIN=target/debug/examples

echo "=== failover run: ${MODE} | ${RATE} msg/s × ${DURATION_S}s, ${KEYS} keys, kill@${KILL_AT_S}s ==="

# 1. build + provision
cargo build -q --example failover_publisher --example load_gen --example output_recorder
LEADER_Q="${LEADER_Q}" INPUT_TOPIC="${INPUT_TOPIC}" bash "${RIG}/provision.sh"

PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill -9 "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

# 2. recorder (truncates OUT_FILE); runs through load + tail
RUN_S=$(( DURATION_S + TAIL_S + 4 )) OUT_FILE="${OUT_FILE}" "${BIN}/output_recorder" >/tmp/failover-rec.out 2>&1 &
REC_PID=$!; PIDS+=("$REC_PID")
sleep 1.5

# 3. replicas A and B on the exclusive queue
"${BIN}/failover_publisher" A >"${A_OUT}" 2>&1 &  A_PID=$!; PIDS+=("$A_PID")
"${BIN}/failover_publisher" B >"${B_OUT}" 2>&1 &  B_PID=$!; PIDS+=("$B_PID")

# 4. wait for exactly one to go ACTIVE, record who leads
LEADER=""; LEADER_PID=""; STANDBY=""; STANDBY_PID=""
for _ in $(seq 1 20); do
  sleep 0.5
  a_act=$(grep -c "A ACTIVE" "${A_OUT}" 2>/dev/null || true); a_act=${a_act:-0}
  b_act=$(grep -c "B ACTIVE" "${B_OUT}" 2>/dev/null || true); b_act=${b_act:-0}
  if [[ "${a_act}" -gt 0 && "${b_act}" -eq 0 ]]; then LEADER=A; LEADER_PID=$A_PID; STANDBY=B; STANDBY_PID=$B_PID; break; fi
  if [[ "${b_act}" -gt 0 && "${a_act}" -eq 0 ]]; then LEADER=B; LEADER_PID=$B_PID; STANDBY=A; STANDBY_PID=$A_PID; break; fi
done
[[ -z "${LEADER}" ]] && { echo "FAIL: no single Active leader after bind" >&2; exit 1; }
echo "leader=${LEADER} (pid ${LEADER_PID}), standby=${STANDBY} (pid ${STANDBY_PID})"

# 5. load under which the failover happens
RATE="${RATE}" DURATION_S="${DURATION_S}" KEYS="${KEYS}" "${BIN}/load_gen" >/tmp/failover-load.out 2>&1 &
LOAD_PID=$!; PIDS+=("$LOAD_PID")

# 6. drop the leader mid-load
sleep "${KILL_AT_S}"
if [[ "${MODE}" == "graceful" ]]; then
  echo ">>> SIGTERM leader ${LEADER} (pid ${LEADER_PID}) @ $(date +%s%3N)"
  kill -TERM "${LEADER_PID}" 2>/dev/null || true
else
  echo ">>> SIGKILL leader ${LEADER} (pid ${LEADER_PID}) @ $(date +%s%3N)"
  kill -9 "${LEADER_PID}" 2>/dev/null || true
fi

# 7. let load finish + recorder drain the tail
wait "${LOAD_PID}" 2>/dev/null || true
sleep "${TAIL_S}"
kill -TERM "${STANDBY_PID}" 2>/dev/null || true
kill -9 "${REC_PID}" 2>/dev/null || true
sleep 1

# 8. results
echo
echo "=== leader timeline (A) ==="; grep -E "ACTIVE|INACTIVE|STOPPED" "${A_OUT}" || true
echo "=== leader timeline (B) ==="; grep -E "ACTIVE|INACTIVE|STOPPED" "${B_OUT}" || true
echo
python3 -I "${RIG}/analyze.py" "${OUT_FILE}" "${LEADER}"
