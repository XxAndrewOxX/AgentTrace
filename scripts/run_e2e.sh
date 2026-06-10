#!/usr/bin/env bash
# Run agent-trace E2E test suites.
#
# Usage:
#   ./scripts/run_e2e.sh              # run all E2E suites (excludes live)
#   ./scripts/run_e2e.sh user         # run user journey tests
#   ./scripts/run_e2e.sh agent        # run agent interaction tests
#   ./scripts/run_e2e.sh permissions  # run permission tests
#   ./scripts/run_e2e.sh recovery     # run failure/recovery tests
#   ./scripts/run_e2e.sh performance  # run performance tests
#   ./scripts/run_e2e.sh tui          # run TUI behavior tests
#   ./scripts/run_e2e.sh adversarial  # run adversarial validation tests
#   ./scripts/run_e2e.sh connection   # run agent connection tests (CLI + MCP)
#   ./scripts/run_e2e.sh unit         # run unit tests only
#   ./scripts/run_e2e.sh live         # run live agent tests (requires GROQ_API_KEY)

set -euo pipefail
cd "$(dirname "$0")/.."

SUITE="${1:-all}"
CARGO_ARGS="${CARGO_ARGS:---release}"

run_suite() {
    local name="$1"
    local test_name="$2"
    echo ""
    echo "══════════════════════════════════════════"
    echo "  $name"
    echo "══════════════════════════════════════════"
    cargo test $CARGO_ARGS --test "$test_name" -- --nocapture 2>&1
}

case "$SUITE" in
    user)
        run_suite "User Journey Tests (UJ-1..6)" "e2e_user_journeys"
        ;;
    agent)
        run_suite "Agent Interaction Tests (AI-1..8)" "e2e_agent_interactions"
        ;;
    permissions)
        run_suite "Permission Tests (PI-1..5)" "e2e_permissions"
        ;;
    recovery)
        run_suite "Failure & Recovery Tests (FR-1..8)" "e2e_failure_recovery"
        ;;
    performance)
        run_suite "Performance Tests (PS-1..5)" "e2e_performance"
        ;;
    tui)
        run_suite "TUI Behavior Tests (TB-1..10)" "e2e_tui_behavior"
        ;;
    adversarial)
        run_suite "Adversarial Validation (35 cases)" "adversarial_validation"
        ;;
    connection)
        run_suite "Agent Connection Tests (AC-1..9, MC-1..11)" "e2e_agent_connection"
        ;;
    live)
        echo ""
        echo "══════════════════════════════════════════"
        echo "  Live Agent Tests (AE-001, AE-004, AE-008)"
        echo "══════════════════════════════════════════"
        if [ -z "${GROQ_API_KEY:-}" ] && [ "${AGENT_TRACE_MODEL_BACKEND:-groq}" = "groq" ]; then
            echo "ERROR: GROQ_API_KEY is not set. Export it or set AGENT_TRACE_MODEL_BACKEND=ollama."
            exit 1
        fi
        AGENT_TRACE_LIVE_TESTS=1 cargo test $CARGO_ARGS --test e2e_live_agent -- --ignored --nocapture 2>&1
        ;;
    unit)
        echo ""
        echo "══════════════════════════════════════════"
        echo "  Unit Tests"
        echo "══════════════════════════════════════════"
        cargo test $CARGO_ARGS --lib -- --nocapture 2>&1
        ;;
    all)
        run_suite "User Journey Tests (UJ-1..6)"        "e2e_user_journeys"
        run_suite "Agent Interaction Tests (AI-1..8)"   "e2e_agent_interactions"
        run_suite "Permission Tests (PI-1..5)"          "e2e_permissions"
        run_suite "Failure & Recovery Tests (FR-1..8)"  "e2e_failure_recovery"
        run_suite "Performance Tests (PS-1..5)"         "e2e_performance"
        run_suite "TUI Behavior Tests (TB-1..10)"       "e2e_tui_behavior"
        run_suite "Adversarial Validation (35 cases)"   "adversarial_validation"
        run_suite "Agent Connection Tests (AC-1..9, MC-1..11)" "e2e_agent_connection"
        ;;
    *)
        echo "Unknown suite: $SUITE"
        echo "Usage: $0 [all|user|agent|permissions|recovery|performance|tui|adversarial|connection|unit|live]"
        exit 1
        ;;
esac

echo ""
echo "Done."
