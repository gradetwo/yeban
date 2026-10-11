#!/bin/bash
# timeout_tail: run a command with a limit; on timeout STILL persist the partial output,
# then classify the timeout with HARNESS PROGRESS keys as the key.
# usage: timeout_tail.sh <seconds> <output-file> <command...>
set -u
LIMIT="$1"
shift
OUT="$1"
shift
: > "$OUT"
"$@" > "$OUT" 2>&1 &
PID=$!
sleep "$LIMIT"
TIMED=0
if kill -0 "$PID" 2>/dev/null; then
  TIMED=1
  kill -TERM "$PID" 2>/dev/null
  sleep 2
  kill -KILL "$PID" 2>/dev/null
fi
wait "$PID" 2>/dev/null
CODE=$?
if [ "$CODE" = "" ]; then
  CODE=0
fi
SIZE=$(wc -c < "$OUT" | tr -d ' ')
K1=$(grep -cE "running [0-9]+ tests?" "$OUT" || true)
K2=$(grep -cE "^test [^ ]+ \.\.\." "$OUT" || true)
K3=$(grep -cE "^test result:" "$OUT" || true)
if [ "$TIMED" = "1" ]; then
  echo "[timeout_tail] TIMEOUT bytes=$SIZE"
  if [ "$K1" -gt 0 ] || [ "$K2" -gt 0 ] || [ "$K3" -gt 0 ]; then
    echo "[timeout_tail] verdict=HARNESS_ALIVE running=$K1 test=$K2 result=$K3 (timeout is TRUSTWORTHY)"
    exit 124
  fi
  echo "[timeout_tail] verdict=HARNESS_DEAD running=$K1 test=$K2 result=$K3 (timeout is NOT trustworthy)"
  exit 125
fi
echo "[timeout_tail] FINISHED exit=$CODE bytes=$SIZE"
exit "$CODE"
