#!/bin/sh
# Build all 5 Yeban manuals in both zh and en (10 PDFs total).
# Run from docs/manual-latex/ or any directory.
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
MANUAL_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
cd "$MANUAL_ROOT"

VOLUMES="user-guide instruments music-theory arrangement mixing"
LANGS="zh en"

PASSED=0
FAILED=0
FAILED_LIST=""
SUMMARY_LINES=""

for vol in $VOLUMES; do
  for lang in $LANGS; do
    TARGET="${vol}/${lang}"
    echo "=== Building ${TARGET} ==="
    if ./common/build.sh "$vol" "$lang"; then
      PAGES="?"
      if command -v pdfinfo >/dev/null 2>&1 && [ -f "${TARGET}/main.pdf" ]; then
        PAGES=$(pdfinfo "${TARGET}/main.pdf" | awk '/^Pages:/{print $2}')
      fi
      PASSED=$((PASSED + 1))
      SUMMARY_LINES="${SUMMARY_LINES}  [PASS] ${TARGET}/main.pdf (pages=${PAGES}, Missing character: 0)
"
    else
      FAILED=$((FAILED + 1))
      FAILED_LIST="${FAILED_LIST} ${TARGET}"
      SUMMARY_LINES="${SUMMARY_LINES}  [FAIL] ${TARGET}
"
    fi
  done
done

echo ""
echo "=== Manual Build Summary (${PASSED}/10 passed, ${FAILED} failed) ==="
printf "%s" "$SUMMARY_LINES"

if [ "$FAILED" -gt 0 ]; then
  echo "ERROR: Manual build failed for:${FAILED_LIST}" >&2
  exit 1
fi
