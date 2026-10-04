#!/usr/bin/env bash
# 从品牌母版 assets/brand/yeban.svg 重新生成全部尺寸/主题的 PNG。
#
# 母版是唯一事实源: 它内含 10 个变体 (深色/浅色 × 512/256/128/64/32),
# 每个变体的 SVG 内部 id 都带尺寸前缀 (d512-* / l32-*), 因此可以安全拆分。
#
# 依赖: rsvg-convert (librsvg)。macOS: brew install librsvg
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BRAND="$REPO/assets/brand"

command -v rsvg-convert >/dev/null 2>&1 || {
  echo "缺少 rsvg-convert。macOS: brew install librsvg" >&2
  exit 2
}

cd "$BRAND"
for theme in dark light; do
  for size in 512 256 128 64 32; do
    src="yeban-$theme-$size.svg"
    [[ -f "$src" ]] || { echo "缺少变体文件 $src (请先拆分母版 yeban.svg)" >&2; exit 1; }
    rsvg-convert -w "$size" -h "$size" "$src" -o "png/yeban-$theme-$size.png"
    printf 'rendered %-22s -> png/yeban-%s-%s.png\n' "$src" "$theme" "$size"
  done
done

echo
echo "完成。共 10 个 PNG, 输出目录: $BRAND/png"
