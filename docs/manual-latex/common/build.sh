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
echo "Built: $DIR/main.pdf"
