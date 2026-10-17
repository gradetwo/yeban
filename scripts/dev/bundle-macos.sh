#!/usr/bin/env bash
# 把 `yeban-app` 装配成 macOS `.app` bundle —— 任务单 #140。
#
# 为什么需要它 (实测背景):
#   裸二进制 (`cargo run` / `target/release/yeban-app`) **没有 bundle** ⇒ 没有 Info.plist
#   ⇒ `NSMicrophoneUsageDescription` 无处可放 ⇒ TCC 只能把麦克风授权挂在"终端"上,
#   而不是挂在"夜半"上。同一个二进制换终端/换机器就可能被拒。本脚本给录音链一个
#   **按应用**的身份 (`CFBundleIdentifier` = dev.yeban.app), 让系统能弹权限框。
#
# 用法:
#   bash scripts/dev/bundle-macos.sh
#
# 产物 (全部落在 `target/` 下, 该目录已被 .gitignore 忽略 ⇒ 不污染工作树):
#   target/Yeban.app/Contents/MacOS/yeban         ← target/release/yeban-app 的副本
#   target/Yeban.app/Contents/Info.plist          ← resources/macos/Info.plist + 版本
#   target/Yeban.app/Contents/Resources/          ← 目前为空, 留给图标/资源
#
# 版本单一事实源:
#   根 `Cargo.toml` 的 `[workspace.package] version` (成员用 `version.workspace = true` 继承)。
#   本脚本**不写死版本**, 而是用 `cargo metadata --no-deps --locked` 问 Cargo 要
#   `yeban-app` 解析后的版本, 再 `plutil -replace` 写进 Info.plist 的两个版本键。
#
# 签名:
#   ad-hoc (`codesign --force --deep --sign -`)。这不是 Developer ID 签名, 不能过
#   Gatekeeper 的公证, 但足以让**本机** TCC 把它当成一个真正的应用。签名失败时
#   **不静默**: 打印 codesign 的原话并以非零退出。
#
# 退出码: 0 = 成功; 1 = 任一环节失败 (plutil -lint / 装配 / codesign / smoke)。
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app_name="Yeban"
executable_name="yeban"
bundle="$repo_root/target/$app_name.app"
template="$repo_root/resources/macos/Info.plist"
built_binary="$repo_root/target/release/yeban-app"
version_placeholder="__YEBAN_VERSION__"

step() { printf '\n[bundle] %s\n' "$*"; }
die() { printf '[bundle][fail] %s\n' "$*" >&2; exit 1; }

# --- 0. 平台与工具 ---------------------------------------------------------
[[ "$(uname -s)" == "Darwin" ]] || die "本脚本只适用于 macOS (uname -s = $(uname -s))"
for tool in cargo python3 plutil codesign install; do
  command -v "$tool" >/dev/null 2>&1 || die "缺少工具: $tool"
done

[[ -f "$template" ]] || die "找不到 Info.plist 模板: $template"

# --- 1. 版本 (单一事实源) ---------------------------------------------------
step "解析版本: cargo metadata --no-deps --locked (根 Cargo.toml [workspace.package].version)"
version="$(
  cargo metadata --format-version 1 --no-deps --locked \
    --manifest-path "$repo_root/Cargo.toml" \
    | python3 -c 'import json, sys
packages = json.load(sys.stdin)["packages"]
print(next(package["version"] for package in packages if package["name"] == "yeban-app"))'
)" || die "cargo metadata 失败, 无法确定版本 (Cargo.lock 过期? 见 --locked)"
[[ -n "$version" ]] || die "解析出的版本为空"
printf '[bundle] version = %s\n' "$version"

# --- 2. 模板本身先过一遍 lint ----------------------------------------------
# 模板里留着 `__YEBAN_VERSION__` 占位符 —— 它是合法字符串, 所以 lint 仍应通过。
# 这条不是装饰: 模板改坏了, 这里立刻红, 而不是等装配完才发现。
if ! template_lint="$(plutil -lint "$template")"; then
  printf '%s\n' "$template_lint" >&2
  die "模板 Info.plist 未通过 plutil -lint"
fi
printf '[bundle] plutil -lint 模板: %s\n' "$template_lint"

# --- 3. 构建 ---------------------------------------------------------------
# `--locked`: 本单不允许改 Cargo.lock (sha256 前后必须一致); 锁一旦过期就应大声失败。
step "cargo build --release -p yeban-app --locked"
cargo build --release -p yeban-app --locked --manifest-path "$repo_root/Cargo.toml"
[[ -x "$built_binary" ]] || die "构建结束但找不到可执行文件: $built_binary"

# --- 4. 装配 ---------------------------------------------------------------
# 先整体删掉旧 bundle: 可重复执行的前提是"上一次的残留不参与这一次"。
step "装配 $bundle"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"

install -m 0755 "$built_binary" "$bundle/Contents/MacOS/$executable_name"
cp "$template" "$bundle/Contents/Info.plist"
plist="$bundle/Contents/Info.plist"

# 两个版本键都从**同一个** $version 写入 ⇒ Info.plist 里不存在第二份版本事实源。
plutil -replace CFBundleShortVersionString -string "$version" "$plist"
plutil -replace CFBundleVersion -string "$version" "$plist"

# --- 5. plutil -lint (装配后的真产物) --------------------------------------
if ! plist_lint="$(plutil -lint "$plist")"; then
  printf '%s\n' "$plist_lint" >&2
  die "装配后的 Info.plist 未通过 plutil -lint"
fi
printf '[bundle] plutil -lint: %s\n' "$plist_lint"

if grep -q -- "$version_placeholder" "$plist"; then
  die "Info.plist 里仍有未替换的版本占位符 $version_placeholder"
fi

short_version="$(plutil -extract CFBundleShortVersionString raw -o - "$plist")"
bundle_version="$(plutil -extract CFBundleVersion raw -o - "$plist")"
[[ "$short_version" == "$version" ]] || \
  die "CFBundleShortVersionString ($short_version) != cargo 版本 ($version)"
[[ "$bundle_version" == "$version" ]] || \
  die "CFBundleVersion ($bundle_version) != cargo 版本 ($version)"
mic_description="$(plutil -extract NSMicrophoneUsageDescription raw -o - "$plist")"
[[ -n "$mic_description" ]] || die "Info.plist 缺少 NSMicrophoneUsageDescription"

# --- 6. ad-hoc 签名 --------------------------------------------------------
# 失败时**如实报**: 打印 codesign 的原始诊断, 不以 0 退出假装成功。
step "ad-hoc 签名: codesign --force --deep --sign -"
if codesign --force --deep --sign - "$bundle"; then
  printf '[bundle] codesign: 成功 (ad-hoc / 无 Developer ID)\n'
else
  sign_rc=$?
  printf '[bundle][fail] codesign 返回 %s; bundle 已装配但**未签名**\n' "$sign_rc" >&2
  die "ad-hoc 签名失败 (原因见上面 codesign 的原话); 未签名 bundle 仍可手动运行"
fi

# --- 7. 冒烟: bundle 内的二进制真的能跑 ------------------------------------
# `--version` 是 CLI 的短路命令 (src/cli.rs), 不初始化 Slint/音频 ⇒ 不会弹窗。
step "smoke: $bundle/Contents/MacOS/$executable_name --version"
version_output="$("$bundle/Contents/MacOS/$executable_name" --version)"
printf '[bundle] %s\n' "$version_output"
case "$version_output" in
  *"$version"*) ;;
  *) die "版本行不含 $version: $version_output" ;;
esac

# --- 8. 报告 ---------------------------------------------------------------
step "产物"
printf 'bundle 路径: %s\n' "$bundle"

total_bytes=0
while IFS=' ' read -r size file_path; do
  printf '  %10s B  %s\n' "$size" "$file_path"
  total_bytes=$((total_bytes + size))
done < <(find "$bundle" -type f -exec stat -f '%z %N' {} + | sort -k2)

bundle_id="$(plutil -extract CFBundleIdentifier raw -o - "$plist")"
printf 'bundle 总字节数: %s B\n' "$total_bytes"
printf 'CFBundleIdentifier: %s\n' "$bundle_id"
printf 'CFBundleShortVersionString: %s\n' "$short_version"
printf 'CFBundleVersion: %s\n' "$bundle_version"
printf 'NSMicrophoneUsageDescription: %s\n' "$mic_description"
