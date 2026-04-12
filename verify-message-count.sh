#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${SCRIPT_DIR}/.env"

if [[ ! -f "$ENV_FILE" ]]; then
  echo "Error: .env file not found at $ENV_FILE"
  echo "Copy .env.example to .env and fill in your connection details."
  exit 1
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

EXPECTED_COUNT="${EXPECTED_COUNT:-1033}"

echo "Building..."
cargo build --example count_messages -q

echo "Starting message counter (expecting ${EXPECTED_COUNT} messages)..."
"${SCRIPT_DIR}/target/debug/examples/count_messages" &
COUNTER_PID=$!

# Give the counter time to connect and subscribe before tests start publishing
sleep 2

echo "Running integration tests..."
cargo test --test integration_test -- --ignored -q 2>&1

echo "Waiting for quiet period to expire..."
wait "$COUNTER_PID"
EXIT_CODE=$?

if [[ $EXIT_CODE -eq 0 ]]; then
  echo "PASS"
else
  echo "FAIL"
  exit 1
fi
