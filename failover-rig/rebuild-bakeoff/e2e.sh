#!/usr/bin/env bash
# e2e.sh <replay|shadow> — END-TO-END rebuild catch-up: drain the backlog from the chosen
# source AND re-feed it into a cold Feldera, timing how long until Feldera's `rolling` view
# is rebuilt to the full backlog. This adds Feldera ingest/rebuild on top of the source drain;
# the source throughput (replay ~65k/s vs shadow ~1.6k/s) is the dominant term.
#
# Requires the continuum-payments broker + docker network. Removes its Feldera container on
# exit (KEEP=1 to leave it). Env: BACKLOG (20000), KEYS (16), SOLACE_BROKER_PASSWORD (default).

set -euo pipefail
MODE="${1:-replay}"; case "$MODE" in replay|shadow) ;; *) echo "usage: e2e.sh <replay|shadow>">&2; exit 2;; esac
cd "$(dirname "$0")"; ROOT=../..; BIN=$ROOT/target/debug/examples
NETWORK="${NETWORK:-continuum-payments-feldera_default}"; IMAGE="${IMAGE:-ghcr.io/jessemenning/feldera:feat-solace-connector}"
PORT=8093; PN=bakeoff-e2e; C=bakeoff-e2e-feldera; FELD_Q=bakeoff-feld-q; FEED_TOPIC=bakeoff/feed
FL="http://localhost:$PORT"
SEMP="${SEMP_BASE:-http://localhost:8089/SEMP/v2/config}"; SU="${SEMP_USER:-admin}"; SP="${SEMP_PASS:-admin}"; VPN="${VPN:-default}"
SRC_TOPIC="${SRC_TOPIC:-bakeoff/src}"; SRC_Q=bakeoff-src-q; SHADOW_Q=bakeoff-shadow-q
BACKLOG="${BACKLOG:-20000}"; KEYS="${KEYS:-16}"
export SOLACE_BROKER_URL="${SOLACE_BROKER_URL:-tcp://localhost:55555}" SOLACE_BROKER_PASSWORD="${SOLACE_BROKER_PASSWORD:-default}"

cleanup(){ [ "${KEEP:-0}" = 1 ] || docker rm -f "$C" >/dev/null 2>&1 || true; }
trap cleanup EXIT
post(){ curl -s -o /dev/null -X POST --user "$SU:$SP" -H "Content-Type: application/json" "$SEMP/msgVpns/$VPN/$1" -d "$2"; }
d(){ curl -s -o /dev/null -X DELETE --user "$SU:$SP" "$SEMP/msgVpns/$VPN/$1" 2>/dev/null || true; }
mkq(){ post "queues" "{\"queueName\":\"$1\",\"accessType\":\"exclusive\",\"ingressEnabled\":true,\"egressEnabled\":true,\"permission\":\"consume\",\"maxMsgSpoolUsage\":4000}"; post "queues/$1/subscriptions" "{\"subscriptionTopic\":\"$2\"}"; }
total(){ curl -s -G "$FL/v0/pipelines/$PN/query" --data-urlencode "sql=SELECT COALESCE(SUM(cnt),0) AS t FROM rolling" --data-urlencode "format=json" 2>/dev/null | python3 -I -c "import json,sys;
import sys
for line in sys.stdin:
  line=line.strip()
  if line:
    print(json.loads(line).get('t',0)); break
else: print(0)" 2>/dev/null || echo 0; }

echo "=== provision (src + shadow + replay log + feld-in queue) ==="
bash provision.sh >/dev/null
d "queues/$FELD_Q"; mkq "$FELD_Q" "$FEED_TOPIC/>"
echo "  ok"

echo "=== start cold Feldera + load program ==="
docker rm -f "$C" >/dev/null 2>&1 || true
docker run -d --name "$C" --network "$NETWORK" -p "$PORT:8080" "$IMAGE" >/dev/null
echo -n "  waiting healthz "; for _ in $(seq 1 60); do curl -sf "$FL/healthz">/dev/null 2>&1 && { echo up; break; }; echo -n .; sleep 2; done
jq -Rs --arg n "$PN" '{name:$n,program_code:.,runtime_config:{workers:4}}' < program-e2e.sql | curl -s -X PUT -H "Content-Type: application/json" -d @- "$FL/v0/pipelines/$PN" >/dev/null
echo -n "  compiling "; for _ in $(seq 1 90); do [ "$(curl -s "$FL/v0/pipelines/$PN"|jq -r .program_status 2>/dev/null)" = Success ] && { echo done; break; }; echo -n .; sleep 2; done
curl -s -X POST "$FL/v0/pipelines/$PN/start" >/dev/null
echo -n "  starting "; for _ in $(seq 1 60); do [ "$(curl -s "$FL/v0/pipelines/$PN"|jq -r .deployment_status 2>/dev/null)" = Running ] && { echo running; break; }; echo -n .; sleep 2; done

echo "=== preload $BACKLOG msgs to $SRC_TOPIC (the pre-crash backlog) ==="
RATE=$(( BACKLOG / 5 )); [ "$RATE" -lt 1 ] && RATE=$BACKLOG
KEYED=1 INPUT_TOPIC="$SRC_TOPIC" RATE="$RATE" DURATION_S=5 KEYS="$KEYS" "$BIN/load_gen" 2>&1 | tail -1
sleep 2

if [ "$MODE" = replay ]; then
  d "queues/$SRC_Q"; mkq "$SRC_Q" "$SRC_TOPIC/>"   # empty so replay comes from the log
  SRCQ="$SRC_Q"; START=BEGINNING
else
  SRCQ="$SHADOW_Q"; START=BEGINNING
fi

echo "=== $MODE rebuild: drain source + re-feed Feldera, timing catch-up to $BACKLOG ==="
t0=$(date +%s.%N)
# Shadow has no clean end-of-drain signal, so give it an authoritative target (the backlog
# size) — otherwise the quiet heuristic silently under-drains under re-feed load. Replay ends
# cleanly on its own (EXPECT_N=0).
EXP=0; [ "$MODE" = shadow ] && EXP=$BACKLOG
SRC_QUEUE="$SRCQ" REPLAY_START="$START" REFEED_TOPIC="$FEED_TOPIC" EXPECT_N="$EXP" "$BIN/rebuild_refeeder" "$MODE" 2>&1 | grep RESULT &
# Poll Feldera until its view sums to the full backlog (rebuilt).
caught=0
for _ in $(seq 1 240); do
  t=$(total); [ "${t:-0}" -ge "$BACKLOG" ] && { caught=$(date +%s.%N); break; }
  sleep 0.5
done
wait 2>/dev/null || true
if [ "$caught" = 0 ]; then
  echo "  DID NOT catch up to $BACKLOG within timeout (last view total=$(total))"
else
  python3 -I -c "print(f'  CATCH-UP ({\"$MODE\"}): Feldera rolling rebuilt to $BACKLOG in {$caught-$t0:.1f}s')"
fi
echo "(KEEP=1 leaves $C up.)"
