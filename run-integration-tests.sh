#!/usr/bin/env bash
# run-integration-tests.sh
#
# Runs all integration tests (sync + async) against a live broker.
#
# Usage:
#   ./run-integration-tests.sh                  # run tests only
#   ./run-integration-tests.sh --setup          # configure broker, then run tests
#   ./run-integration-tests.sh --setup --teardown  # configure, run, then tear down
#
# Broker credentials are read from .env.
# --setup / --teardown also require SEMP_USERNAME and SEMP_PASSWORD in .env
# (or exported in the shell).  These are the management credentials found in
# the Solace Cloud console: Connect → SEMP REST API.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${SCRIPT_DIR}/.env"

# ---------------------------------------------------------------------------
# Parse flags
# ---------------------------------------------------------------------------
DO_SETUP=false
DO_TEARDOWN=false
for arg in "$@"; do
  case "${arg}" in
    --setup)    DO_SETUP=true ;;
    --teardown) DO_TEARDOWN=true ;;
    *) echo "Unknown argument: ${arg}"; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Load .env
# ---------------------------------------------------------------------------
if [[ ! -f "${ENV_FILE}" ]]; then
  echo "Error: .env not found at ${ENV_FILE}"
  echo "Copy .env.example to .env and fill in your connection details."
  exit 1
fi
set -a; source "${ENV_FILE}"; set +a

echo "==> Broker: ${SOLACE_BROKER_URL:-<unset>}"
echo

# ---------------------------------------------------------------------------
# Optional: configure broker before running tests
# ---------------------------------------------------------------------------
if [[ "${DO_SETUP}" == true ]]; then
  echo "==> Running broker setup ..."
  "${SCRIPT_DIR}/scripts/configure-broker.sh"
  echo
fi

# ---------------------------------------------------------------------------
# Run tests
# ---------------------------------------------------------------------------
echo "==> Running sync integration tests ..."
cargo test --test integration_test -- --ignored

echo
echo "==> Running async integration tests ..."
cargo test --features async --test async_integration_test -- --include-ignored

echo
echo "==> All tests complete."

# ---------------------------------------------------------------------------
# Optional: tear down after tests
# ---------------------------------------------------------------------------
if [[ "${DO_TEARDOWN}" == true ]]; then
  echo
  echo "==> Running broker teardown ..."
  "${SCRIPT_DIR}/scripts/teardown-broker.sh"
fi
