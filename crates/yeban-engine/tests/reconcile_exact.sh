#!/bin/bash
# R181 自动对账：对每个判据核 "passed == 1" 且 "passed + filtered out == 目标内判据总数"。
#   ⛔ 不写死期望值（总数当场从 --list 算）；⛔ 不只核等式（拼错的名字会给出 0 passed 而等式仍成立）。
#   用法: reconcile_exact.sh <test-target> <criterion>...
set -u
TARGET="$1"; shift
[ -n "$TARGET" ] || { echo "usage: $0 <test-target> <criterion>..."; exit 2; }
# 目标内判据总数：当场算
TOTAL=$(cargo test -p yeban-engine --no-default-features --test "$TARGET" -- --list 2>/dev/null | grep -c ": test$")
echo "[reconcile] 目标=$TARGET 目标内判据总数(当场算)=$TOTAL"
FAIL=0
for NAME in "$@"; do
  OUT=$(cargo test -p yeban-engine --no-default-features --test "$TARGET" -- --exact "$NAME" 2>&1)
  CODE=$?
  LINE=$(printf '%s\n' "$OUT" | grep -E "^test result" | head -1)
  PASSED=$(printf '%s\n' "$LINE" | sed -n 's/.*ok\. \([0-9]*\) passed.*/\1/p')
  FILTERED=$(printf '%s\n' "$LINE" | sed -n 's/.* \([0-9]*\) filtered out.*/\1/p')
  [ -n "$PASSED" ] || PASSED=-1; [ -n "$FILTERED" ] || FILTERED=-1
  echo "[reconcile] $NAME: passed=$PASSED filtered=$FILTERED cargo_exit=$CODE"
  # ⭐ 必须同时核三件事：① passed == 1（拼错名字 ⇒ 0）② 等式 ③ cargo 退出码
  if [ "$PASSED" != "1" ]; then echo "[reconcile] ✗ passed != 1 ⇒ 判据名可能拼错"; FAIL=1; continue; fi
  if [ "$((PASSED + FILTERED))" != "$TOTAL" ]; then echo "[reconcile] ✗ $PASSED + $FILTERED != $TOTAL"; FAIL=1; continue; fi
  if [ "$CODE" != "0" ]; then echo "[reconcile] ✗ cargo_exit != 0"; FAIL=1; continue; fi
  echo "[reconcile] ✓ $PASSED + $FILTERED == $TOTAL 且 passed == 1"
done
[ "$FAIL" = "0" ] || { echo "[reconcile] 对账失败"; exit 1; }
echo "[reconcile] 全部对账通过"
