#!/usr/bin/env bash
# ⭐ #172: 打印 `$APP_FEATURES` 里 `crates/<crate>/Cargo.toml` **真的声明**的那些特性。
#
# 为什么需要它: 合并后的 clippy / test 两条特性腿对**每个** matrix crate 都会跑,
# 而 `--features` 里只要有一个该 crate 没声明的名字, cargo 就直接报错退出
# （R64 / R335 的原话）。合并前每条腿各自用 `grep -q '^<feature>' crates/<crate>/Cargo.toml`
# 守卫同一件事; 现在清单只写一次（workflow 级 `APP_FEATURES`）、过滤也只写一次（这里）
# ⇒ 清单增删不会在两处漂移。
#
# 用法: APP_FEATURES=a,b,c bash scripts/dev/app-features.sh <crate>
# 输出: 逗号分隔的「该 crate 已声明」的特性（可能为空 ⇒ 调用方跳过该腿）。
#
# bash 3.2 兼容（开发机自带 3.2）: 不用数组、不用关联数组 —— `policy_check.py` 的 G14 会查。
set -euo pipefail

crate="${1:?用法: app-features.sh <crate>}"
manifest="crates/$crate/Cargo.toml"
if [ ! -f "$manifest" ]; then
  echo "找不到 $manifest" >&2
  exit 2
fi

list="${APP_FEATURES:-}"
if [ -z "$list" ]; then
  echo "APP_FEATURES 未设置或为空 —— 它应由 workflow 的 env 传入" >&2
  exit 2
fi

out=""
rest="$list"
while [ -n "$rest" ]; do
  case "$rest" in
    *,*) feature="${rest%%,*}"; rest="${rest#*,}" ;;
    *)   feature="$rest"; rest="" ;;
  esac
  # 容忍清单里手写的空格（`a, b`），但输出永远是规范的无空格形式。
  feature="${feature# }"; feature="${feature% }"
  [ -n "$feature" ] || continue
  # TOML 的 feature 键在本仓全部是裸键（行首即名字，后跟 `=`）。
  # 用 `^<name>[[:space:]]*=` 而不是 `^<name>` —— 后者会连注释或前缀相同的名字一起命中。
  if grep -qE "^${feature}[[:space:]]*=" "$manifest"; then
    out="${out:+$out,}$feature"
  fi
done
printf '%s' "$out"
