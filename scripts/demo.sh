#!/usr/bin/env bash
# Heirloom 90-second demo.
#
# Spins up a fresh, throwaway Heirloom store; seeds it with a handful of
# example notes; runs three illustrative searches; prints results.
#
# Run from the repo root after `cargo build --release`:
#   ./scripts/demo.sh

set -euo pipefail

HEIRLOOM="${HEIRLOOM:-./target/release/heirloom}"
[ -x "$HEIRLOOM" ] || { echo "build first: cargo build --release"; exit 1; }

DEMO_HOME="$(mktemp -d -t heirloom-demo.XXXXXX)"
DEMO_NOTES="$(mktemp -d -t heirloom-notes.XXXXXX)"
trap "rm -rf $DEMO_HOME $DEMO_NOTES" EXIT

export HEIRLOOM_HOME="$DEMO_HOME"

cat > "$DEMO_NOTES/auth.md" <<'EOF'
# Auth refactor

Refactoring the OAuth flow to use PKCE. Sam is reviewing.
Deadline is Friday before the demo.
EOF

cat > "$DEMO_NOTES/q2.md" <<'EOF'
# Q2 planning

Three priorities: ship the dashboard, hire one designer, fix latency.
Latency target: p95 under 200ms.
EOF

cat > "$DEMO_NOTES/reading.md" <<'EOF'
# Reading list

- Designing Data-Intensive Applications — Kleppmann
- The Soul of A New Machine — Kidder
- Seeing Like a State — Scott
EOF

bar() { printf "\n\033[1;35m── %s ─────────────────────────────────────\033[0m\n" "$1"; }

bar "heirloom init"
"$HEIRLOOM" init

bar "heirloom ingest fs --path \$NOTES"
"$HEIRLOOM" ingest fs --path "$DEMO_NOTES"

bar "heirloom search 'the auth bug Sam was reviewing'"
"$HEIRLOOM" search "the auth bug Sam was reviewing" -k 3

bar "heirloom search 'q2 priorities'"
"$HEIRLOOM" search "q2 priorities" -k 3

bar "heirloom search 'distributed systems book'"
"$HEIRLOOM" search "distributed systems book" -k 3

bar "heirloom status"
"$HEIRLOOM" status
