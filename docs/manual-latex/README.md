# Yeban manuals (LaTeX)

Five volumes, Chinese and English first. Other languages start only
after Wang says so.

## Layout

- `common/yeban-manual.sty` — shared style (XeLaTeX; zh uses ctex).
- `common/build.sh <volume> <zh|en>` — build one PDF.
- `common/glossary-zh.tex`, `common/glossary-en.tex` — shared glossary seed.
- `<volume>/zh/`, `<volume>/en/` — `main.tex` plus `chapters/`.

Volumes: `user-guide`, `instruments`, `music-theory`, `arrangement`,
`mixing`.

## Conventions

- Concept figures: TikZ, shipped with the source.
- UI screenshots: `\screenshot{name}{caption}` placeholder plus a shot
  list in the delivery receipt; management captures and fills
  `figures/` centrally.
- Every chapter ends with a help-topic mapping table (section → help
  anchor, lowercase hyphenated English) for embedding into the in-app
  help system.
- Product claims must match the live tip. Unimplemented features are
  omitted or marked as not yet available.
- Writers deliver source only; management compiles PDFs on a TeX
  machine and reports missing packages instead of forking the style.
