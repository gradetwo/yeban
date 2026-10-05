#!/usr/bin/env bash
# MUST-GATE-005：GPLv3 源码分发包完备性（`.slint` + 确定性 `Cargo.lock` + `cargo vendor` 离线依赖）。
#
# 规范原文（路线图 :381）: "发布源码包必须完整包含所有 `.slint` 声明式源文件、确定性 `Cargo.lock`
# 与 `cargo vendor` 离线依赖"。
#
# 本脚本两部分:
#   (轻) 不联网: ① 每个 `.slint` 都被 git 跟踪 ② `Cargo.lock` 确定（`--locked` 可解） ③ 指向 vendor 的配置存在且自洽;
#   (重) `--full`: 真跑 `cargo vendor --locked` 到临时目录（**联网**、体积大）⇒ 只在手动档跑。
# 用法: `bash scripts/gates/check_vendor.sh [--full]`
set -uo pipefail

repo_root="$(git rev-parse --show-toplevel)" || exit 2
cd "$repo_root"
full=0
[ "${1:-}" = "--full" ] && full=1
fail=0
unknown=0
# ⚠ 先把 cargo 弄到 PATH 上。**第一版就栽在这里**: 脚本直接 `cargo metadata --locked`，而 cargo 不在 PATH 上
# ⇒ `if` 失败 ⇒ 报 FAIL "Cargo.lock 不一致"。**那是假红**：把"工具不可用"说成"锁坏了"，
# 而假红和假绿一样有害（它会让人开始忽略门禁）。⇒ 工具缺失一律记 **unknown（exit 2）**，绝不记 FAIL。
if [ -f scripts/dev/local-env.sh ]; then
  # shellcheck disable=SC1091
  . scripts/dev/local-env.sh >/dev/null 2>&1 || true
fi
if ! command -v cargo >/dev/null 2>&1; then
  printf '[unknown] 找不到 cargo —— 本环境无法判定 %s（不是失败）\n' "MUST-GATE-005"
  exit 2
fi
note() { printf '%s\n' "$*"; }
bad() { printf 'FAIL %s\n' "$*" >&2; fail=1; }

# ① 所有 .slint 都必须被 git 跟踪（声明式源文件缺一个，源码包就不完整）
untracked=0
while IFS= read -r f; do
  git ls-files --error-unmatch "$f" >/dev/null 2>&1 || { bad ".slint 未被 git 跟踪: $f"; untracked=$((untracked+1)); }
done < <(find crates -name '*.slint' -type f 2>/dev/null)
slint_total=$(find crates -name '*.slint' -type f | wc -l | tr -d ' ')
[ "$untracked" -eq 0 ] && note "[ok] .slint 声明式源文件全部被跟踪: ${slint_total} 个"

# ② Cargo.lock 必须确定（能在 --locked 下解析）
if cargo metadata --locked --format-version 1 >/dev/null 2>&1; then
  note "[ok] Cargo.lock 确定: cargo metadata --locked 通过"
else
  bad "Cargo.lock 与清单不一致（--locked 失败）—— 发布包不可复现"
fi

# ③ 若仓库声明了 vendor 目录，配置必须真的指向它并且是可离线解析的
if [ -d vendor ]; then
  if ! grep -rqs 'directory *= *"vendor"' .cargo/config.toml .cargo/config 2>/dev/null; then
    bad "存在 vendor/ 但 .cargo/config.toml 没有指向它 —— 离线包不会被使用"
  else
    note "[ok] vendor/ 存在且 .cargo/config.toml 指向它"
  fi
  if cargo metadata --offline --locked --format-version 1 >/dev/null 2>&1; then
    note "[ok] 离线解析通过: cargo metadata --offline --locked"
  else
    bad "vendor/ 存在但 --offline 解析失败 —— 离线包不完整"
  fi
else
  note "[skip] 仓库内没有 vendor/（发布时由打包流程产出；--full 可验证它能被产出）"
fi

# ④ (重) 真跑 cargo vendor
if [ "$full" -eq 1 ]; then
  tmp=$(mktemp -d)
  note "[run] cargo vendor --locked -> $tmp（联网, 体积大）"
  if cargo vendor --locked "$tmp" >/dev/null 2>&1; then
    count=$(find "$tmp" -maxdepth 1 -mindepth 1 -type d | wc -l | tr -d ' ')
    size=$(du -sh "$tmp" 2>/dev/null | awk '{print $1}')
    note "[ok] cargo vendor 成功: ${count} 个 crate, ${size}"
  else
    bad "cargo vendor --locked 失败 —— 源码包无法离线构建"
  fi
  rm -rf "$tmp"
else
  note "[skip] 未跑 cargo vendor（需联网与时间; 用 --full 或手动档）"
fi

if [ "$fail" -eq 0 ]; then
  note "check_vendor: 通过（MUST-GATE-005 的可机械判定部分）"
  exit 0
fi
exit 1
