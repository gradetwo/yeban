#!/usr/bin/env bash
# 裁剪 Cargo 构建目录里"可重建、且不重建编译缓存"的那几类产物。
#
# ## 为什么需要它
#
# `scripts/dev/premerge-gate.sh` 会为**每一套特性组合**各跑一遍测试
# （`--all-features`、`in-process-mcp`、`实验性导出`、`midi-input`、
# `experimental-logic-import` …）。每套组合都会**链接**一份自己的测试二进制，
# 而同一个测试因此在磁盘上有 5–10 个副本。
#
# 2026-10-22 的实测：`target/debug/deps` 里 **1264 个**已链接二进制合计 **55 G**
# （平均 44 M/个），而 `.rlib` 只有 **4.7 G** —— 大头是**链接产物**，不是编译缓存。
#
# ## 为什么删这些是安全的
#
# | 类别 | 删了之后 |
# | :--- | :--- |
# | `*.rcgu.o`（中间对象） | 下次编译要重编那些 crate；已链接的二进制不受影响 |
# | `deps/` 下**无扩展名**的文件（已链接的测试/示例二进制） | 下次跑测试要**重新链接**；编译缓存（`.rlib`/`.rmeta`）仍在 ⇒ ⛔ 不是全量重编 |
# | `examples/`（示例程序） | 重编即可，产品不用 |
# | `incremental/`（增量缓存） | 只影响增量编译速度 |
#
# ⚠ **⛔ 本脚本不删 `deps` 里的 `.rlib`/`.rmeta`/`.d`** —— 删那些就是**全量重编**。
#
# ## 用法
#
#     bash scripts/dev/trim-target.sh          # 默认：主工作树的 target
#     bash scripts/dev/trim-target.sh --dry    # 只报大小、不删
#     bash scripts/dev/trim-target.sh <dir>    # 指定 target 目录（例如某棵 worktree）
#
# 有 cargo/rustc 在跑时**直接退出**（删除正在写的产物会把构建搞坏）。
set -u

dry=0
target=""
for arg in "$@"; do
    case "$arg" in
        --dry) dry=1 ;;
        *) target="$arg" ;;
    esac
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
: "${target:=$repo_root/target}"

if [ ! -d "$target" ]; then
    echo "trim-target: 没有 $target，无需裁剪"
    exit 0
fi

# ⚠ `pgrep -f` 会把**本脚本自己的命令行**也匹配上（它含 "cargo"/"rustc" 字样），
# 所以按**进程名**精确匹配，并排除本进程与父进程。
running=""
for pid in $(pgrep -x cargo 2>/dev/null) $(pgrep -x rustc 2>/dev/null); do
    [ "$pid" = "$$" ] && continue
    [ "$pid" = "$PPID" ] && continue
    running="$running $pid"
done
if [ -n "$running" ]; then
    echo "trim-target: ⛔ 有 cargo/rustc 在跑（pid:${running}）⇒ 不裁（删除正在写的产物会搞坏构建）" >&2
    exit 1
fi

before="$(du -sm "$target" 2>/dev/null | awk '{print $1}')"
df_before="$(df -h "$target" | awk 'NR==2 {print $4}')"

# 1) 中间对象
rcgu_n="$(find "$target" -name '*.rcgu.o' -type f 2>/dev/null | wc -l | tr -d ' ')"
# 2) 已链接的测试/示例二进制：deps 顶层、无扩展名、且是**可执行**文件
linked_list="$(mktemp)"
find "$target/debug/deps" -maxdepth 1 -type f ! -name '*.*' -perm -u+x 2>/dev/null > "$linked_list"
linked_n="$(wc -l < "$linked_list" | tr -d ' ')"
# 3) 示例程序目录
has_examples=0
[ -d "$target/debug/examples" ] && has_examples=1
# 4) 增量缓存
has_incremental=0
[ -d "$target/debug/incremental" ] && has_incremental=1

echo "trim-target: $target"
echo "  裁剪前: ${before} MB（磁盘可用 ${df_before}）"
echo "  *.rcgu.o           : ${rcgu_n} 个文件"
echo "  已链接测试/示例二进制: ${linked_n} 个"
echo "  examples/          : ${has_examples}"
echo "  incremental/       : ${has_incremental}"

if [ "$dry" -eq 1 ]; then
    echo "  --dry ⇒ 什么都不删"
    rm -f "$linked_list"
    exit 0
fi

find "$target" -name '*.rcgu.o' -type f -delete 2>/dev/null
while IFS= read -r bin; do
    rm -f "$bin" 2>/dev/null
done < "$linked_list"
rm -f "$linked_list"
[ "$has_examples" -eq 1 ] && rm -rf "$target/debug/examples"
[ "$has_incremental" -eq 1 ] && rm -rf "$target/debug/incremental"

after="$(du -sm "$target" 2>/dev/null | awk '{print $1}')"
df_after="$(df -h "$target" | awk 'NR==2 {print $4}')"
echo "  裁剪后: ${after} MB（磁盘可用 ${df_after}）"
echo "  释放:   $(( before - after )) MB"
echo "  ⚠ 下次跑测试会重新**链接**（⛔ 不是重新编译：.rlib/.rmeta 都在）"
