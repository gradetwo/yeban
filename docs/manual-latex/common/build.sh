#!/bin/sh
# Build one manual volume: ./build.sh <volume> <zh|en>
# Requires XeLaTeX (TeX Live with ctex for zh). Run from docs/manual-latex/.
set -eu
VOL="${1:?volume slug required}"
LANG_DIR="${2:?zh or en required}"
DIR="$VOL/$LANG_DIR"
cd "$DIR"
export TEXINPUTS="../../common:${TEXINPUTS:-}"
xelatex -interaction=nonstopmode -halt-on-error main.tex
makeindex main.idx || true
xelatex -interaction=nonstopmode -halt-on-error main.tex
xelatex -interaction=nonstopmode -halt-on-error main.tex

MISSING=$(grep -c "Missing character" main.log || true)
if [ "$MISSING" -gt 0 ]; then
  echo "ERROR: $MISSING Missing character(s) in $DIR/main.log" >&2
  grep "Missing character" main.log >&2 || true
  echo "Build failed: fix the missing glyphs above; do not ship a PDF with blank characters." >&2
  exit 1
fi

echo "Built: $DIR/main.pdf (Missing character: 0)"