#!/usr/bin/env bash
# 夜半门禁执行器 (Gate runner)。
#
# 规范来源: AGENTS.md §3 (DoD), docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md §5
#          docs/DEV_WORKFLOW.md (本机轻量 / CI 全量的分工)
#
# 三种档位:
#   light          格式 + 机械红线守卫。零编译, 任何机器都能跑。
#   crate <name>   在 light 基础上, 对**指定 crate** 跑 clippy + test。
#                  若该 crate 引入了重依赖 (slint/cpal/symphonia/...), 本机档位会拒绝,
#                  必须交给 CI —— 这正是「开发与测试解耦、异步」的落点。
#   full           workspace 全量门禁 (fmt/clippy/test/deny/schema/guards)。
#                  只在 CI 上跑; 本机执行需显式 YEBAN_ALLOW_HEAVY=1 才放行。
#
# 绝不接受管道化的门禁 (SKILL 规则 4): 所有命令的退出码都被显式检查,
# 任何一步红就立刻以非零码退出, 不允许 `| tail` 吞掉失败。
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"

# 环境自适应 (受限沙箱里自动切换 CARGO_HOME/RUSTUP_TOOLCHAIN; 普通终端不改任何东西)。
# 之前这里直接调 `cargo`, 于是工作线在本机跑门禁时会卡在 fmt 那一步报权限错误 ——
# 那属于"工具摩擦变成阻塞", 必须消灭。
# shellcheck source=../dev/local-env.sh
source "$REPO/scripts/dev/local-env.sh"

MODE="${1:-light}"
shift || true

# 同时匹配两种写法 (AGENTS.md 推荐的是点号继承写法, 只匹配 `=` 会漏):
#   cpal = { workspace = true, optional = true }
#   cpal.workspace = true
HEAVY_RE='^[[:space:]]*(slint|slint-build|i-slint-[a-z-]*|cpal|symphonia|rubato|rayon|clack|nih-plug|vst3-sys|zip|flate2|hound|midly|midir|notify|rstar|signalsmith-stretch)([[:space:]]*\.workspace[[:space:]]*|[[:space:]]*)='

step() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }
fail() { printf '\033[31mFAIL\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[32mok\033[0m   %s\n' "$*"; }

run() {
  local label="$1"; shift
  "$@" || fail "$label (exit=$?)"
  ok "$label"
}

gate_fmt() {
  step "cargo fmt --all --check"
  run "fmt" cargo fmt --all --check
}

gate_guards() {
  step "机械红线守卫 (scripts/guards/policy_check.py)"
  run "guards" python3 scripts/guards/policy_check.py
}

gate_docs() {
  step "文档契约（链接 / 规范 ID / ID 字典 / 决策清单 / 门禁状态表 / 阶段状态表 / 三方对齐矩阵）"
  # ⚠ 每一条都必须走 `run` —— 裸调用会被**后面成功的命令**屏蔽。
  # 实测（由 `line/phase-status` 发现并复现）: 在 `check_gate_status.py` 之后插一条 `sys.exit(3)`,
  # `light` 仍 **EXIT=0** 且打印"门禁通过" —— 因为本函数只有 `set -uo pipefail`（无 `-e`）,
  # 返回值等于**最后一条命令**的退出码 ⇒ **三条守卫根本没能阻断门禁**（与该文件头部的自我承诺矛盾, L12 同族）。
  # ⇒ 凡是"守卫", 都必须把自己的失败**记进计数器**, 而不是靠 shell 的返回码传递。
  run "spec-ids" python3 scripts/gates/spec_id_audit.py --check
  run "id-dictionary" python3 scripts/gates/id_dictionary_audit.py
  run "decisions" python3 scripts/gates/check_decisions.py
  run "gate-status" python3 scripts/gates/check_gate_status.py
  run "phase-status" python3 scripts/gates/check_phase_status.py
  # 三方对齐矩阵（系统 / UI / MCP）：漏点名一个工具或方法、发明一个名字、汇总与逐行不符都变红。
  run "feature-alignment" python3 scripts/gates/check_feature_alignment.py
  run "docs" python3 scripts/gates/check_docs_links.py
  run "clippy-changed" bash scripts/gates/clippy-changed.sh
  run "handoff-snapshot" python3 scripts/gates/check_handoff_snapshot.py
  run "diagnostics-single-impl" python3 scripts/gates/check_diagnostics_single_implementation.py
  run "viewport-bounds-wiring" python3 scripts/gates/check_viewport_bounds_wiring.py
  # Linux Tier-1 golden 清单（`crates/yeban-app/tests/golden/linux/MANIFEST.txt`）: 表里那 5 行
  # `filename / sha256 / bytes` 是**手工抄写**的, 仓库里没有任何生成器写它 ⇒ 抄错不会让任何
  # Rust 测试变红（`assert_matches_golden` 只读 PNG, 从不读 MANIFEST）。这条判据拿表逐行核对
  # **磁盘上的真实字节**（存在性 + sha256 + 字节数 + 无未登记文件 + 无重复行 + 表头/出处形状）。
  # 实测代价（本机 M2, 5 张各 6,222,418 字节）: 0.05 s ⇒ 属"零编译、任何机器都能跑"的 light 一族。
  # ⚠ 它是**门禁脚本**, 不是 policy guard: 不占 `G01..G14` 的编号, 「14 条守卫」不受影响。
  run "golden-manifest(linux)" python3 scripts/gates/check_golden_manifest.py
  # `yeban-mcp` 的依赖方向规则（它自己的 Cargo.toml 写着"不拖音频栈进 MCP"）。
  run "mcp-dependency-direction" python3 scripts/gates/check_mcp_dependency_direction.py
  # 纪律检查表门（账本 R261「五组」里**可无歧义机械化**的 3 条）:
  #   C1  `scripts/**` 里真代码含 `git commit` 的脚本必须由显式开关（--commit / DRY_RUN）把关
  #   C3  ci.yml 的上传步必须保留 `if-no-files-found: error`（⛔ 退回 `warn` 会让"没产出"变成绿）
  #   E1  ci.yml 按 crate 拆的 test 步必须带 `--no-fail-fast`（判据内部首败仍即停 ⇒ 拆分才是 N/N 的唯一机制）
  # 其余 24 条**如实登记为 non-mechanical**（需人读 + 各线自证），⛔ 不假装能机械化。
  # ⚠ 为什么只收 3 条: **门一旦误红就会被忽略** ⇒ 只收零假阳性的判据。这条纪律当场兑现过:
  # 该门首版直接在原文上 grep, 把 `worktree.sh` heredoc 里的**提示文字**当成真调用 ⇒
  # 已知绿喂变红（= A5/R264① 的"语料太宽"）⇒ 修法是扫前剥 heredoc 正文与整行注释。
  run "discipline-checklist" python3 scripts/gates/check_discipline_checklist.py
}

gate_schemas() {
  step "JSON Schema 契约校验"
  # `--repo-assets` 是 MUST-GATE-014 的**本体**, 不是可选装饰:
  # 它逐份校验 `assets/**/manifest.json` 的结构、逐条执行 `licence_whitelist`(含与根清单
  # `allowed_licenses` 的交叉对账 / `commercial_usable`), 并逐项重算 SHA-256 与 size_bytes。
  # ⚠ 此前本机 `full` 调的是**不带**该开关的版本 ⇒ 这条判据**只**存在于 CI
  # (`ci.yml:172` / `release.yml:297`), 本机 full 全绿也可以带着一份坏清单。
  # 实测代价 (本机 M2, 各 3 次): 不带 = 0.26 s (4 份 schema); 带 = 0.93 / 0.94 / 1.15 s
  # (8 条 `[ok]`, 多校验 `assets/**` 下 **4** 份清单 —— 根指针 + brand/models/samples ——
  #  其中 `assets/samples/manifest.json` 一条就含 20 594 项) ⇒ 约 +0.7 s。
  # 为什么**不**下放到 `light`: light 档的定位是"零编译, 任何机器都能跑", 而 schema 一族
  # 本来就只跑在 full(light 连 `gate_schemas` 都不调用); 把一条要求 8.9 MB 清单在场 +
  # `jsonschema` 依赖的判据塞进 light, 换来的不是安全, 而是"在没取回素材的机器上变成环境错误"。
  # 宁可慢一拍也不静默跳过 ⇒ 只在真正跑 schema 的 full 档加开关。
  run "schemas" python3 scripts/gates/validate_schemas.py --repo-assets
}

# 依赖许可清单漂移检查: 需要 cargo metadata (不编译, 只解析), 因此属于"零编译"一族。
# 为什么放进 light 档: 工作线反馈"加依赖 → 本机全绿 → CI 红"必然复现 —— 因为这条判据
# 只在 CI 的 checks job 里跑。一条判据如果本机能跑却只放在 CI, 就会制造无谓的红。
# 直接跑在 light 档里, 让它在提交前就能发现。
gate_license_inventory() {
  step "依赖许可清单漂移检查"
  run "licenses" python3 scripts/gates/license_inventory.py --check
}

gate_deny() {
  step "cargo deny check (开源合规)"
  # 优先用 PATH 里的 cargo-deny; 也可以用预编译二进制并通过 YEBAN_CARGO_DENY 指过来
  # (docs/DEV_WORKFLOW.md: 不要为了装它在本机做一次重编译)。
  local deny_bin="${YEBAN_CARGO_DENY:-$(command -v cargo-deny || true)}"
  if [[ -z "$deny_bin" ]]; then
    fail "cargo-deny 未安装 (CI 上由 EmbarkStudios/cargo-deny-action 提供; 本机见 docs/DEV_WORKFLOW.md)"
  fi
  run "cargo-deny" "$deny_bin" --all-features check
}

#: 这些 crate 的**重依赖挂在 feature 后面** ⇒ 本机可以用 `--no-default-features` 真编译真跑
#: (ADR-0001 D19)。比"整条跳过"更有价值: 仍然是零重依赖的本机验证, 但仍然不跑重活。
#:
#: ⚠ 刻意**不用关联数组**: 开发机 macOS 自带的是 **bash 3.2**, 它没有 `declare -A`
#: （实测: `${ARR[key]}` 会被当成**算术下标**, 于是 `key` 里的 `-` 让 bash 报
#: `yeban: unbound variable`, 而且 `bash -n` 语法检查**照样通过** —— 只有真跑才暴露）。
#: 用 `case` 表达同一件事, 在 bash 3.2 与 5.x 上都成立。
light_variant_of() {
  case "$1" in
    yeban-engine) printf '%s' "--no-default-features" ;;
    *) return 1 ;;
  esac
}

heavy_deps_of() {
  # ⚠ 必须看**传递**依赖, 不能只看本 crate 的清单。
  #
  # 为什么: `yeban-mcp` 自己的清单里没有任何重依赖, 但它现在依赖 `yeban-render`
  # ⇒ 传递拉进 rayon/hound/midly。旧实现只看本 crate 清单 ⇒ 本机会**真的编译**这些重依赖,
  # 直接违背"本机不跑重活、CI 在 GitHub 上跑"的纪律(人类负责人第 7 轮重申)。
  # 判定逻辑住在 `scripts/dev/heavy-deps.py`(可单独测): 0=含重依赖 / 1=不含 / 2=无法判定。
  # 由 `line/mcp-render` 的 needs-7 发现, 集成者落地。
  python3 scripts/dev/heavy-deps.py "$1" >/dev/null 2>&1
  [[ $? -ne 1 ]]   # 0 或 2 都按"含重依赖"处理(2 = 拿不到图, 保守跳过)
}

legacy_heavy_deps_of() {
  local crate="$1"
  # ① 先用清单里的直接匹配快速命中(绝大多数情况到此为止, 不启动 cargo)
  local manifest="crates/$1/Cargo.toml"
  [[ -f "$manifest" ]] || manifest="spikes/$1/Cargo.toml"
  [[ -f "$manifest" ]] || return 1
  if grep -E "$HEAVY_RE" "$manifest" 2>/dev/null | grep -v '^[[:space:]]*#' | grep -q .; then
    return 0
  fi
  # ② 再看依赖图(resolve 图, 不编译任何东西; 只在 ① 没命中时才走这条路)
  local metadata
  metadata="$(cargo metadata --format-version 1 --no-deps 2>/dev/null)" || return 1
  python3 - "$crate" <<'PY' <<<"$metadata"
import json, sys
crate = sys.argv[1]
heavy = ("slint", "cpal", "winit", "symphonia", "rubato", "rayon", "hound", "midly")
data = json.load(sys.stdin)
# 用**本 crate 的直接依赖名**再走一层: `--no-deps` 只给成员自己的清单,
# 所以对每个直接依赖查它的清单(成员 → 一次即可覆盖 crates/* 之间的边)。
members = {pkg["name"]: pkg for pkg in data["packages"]}
pkg = members.get(crate)
if pkg is None:
    sys.exit(1)
names = {dep["name"] for dep in pkg["dependencies"]}
sys.exit(0 if names & set(heavy) else 1)
PY
}

gate_crate() {
  local crate="$1" extra=()
  if [[ -z "${YEBAN_ALLOW_HEAVY:-}" ]] && heavy_deps_of "$crate"; then
    if variant="$(light_variant_of "$crate")"; then
      # 重依赖在 feature 后面 ⇒ 跑**本机轻量变体**(不编译重依赖), 而不是整条跳过。
      extra=("$variant")
      printf '\033[33mNOTE\033[0m %s 的 %s 变体零重依赖, 本机用该变体真跑 (D19)\n' "$crate" "$variant"
    else
      printf '\033[33mSKIP\033[0m %s 含重依赖, 本机不编译 (交给 GitHub CI; 见 docs/DEV_WORKFLOW.md)\n' "$crate"
      return 0
    fi
  fi
  step "crate $crate: clippy --all-targets -D warnings ${extra[*]:-}"
  # ⚠ `"${extra[@]}"` 在 **bash 3.2 + set -u** 下、当数组为空时会报 `extra[@]: unbound variable`
  # （实测: 开发机 /bin/bash 是 3.2 ⇒ **除 yeban-engine 外所有 crate 的本机 crate 档都跑不起来**）。
  # CI 的 bash 5 不受影响, 所以这个 bug 只在本地暴露 —— 由 `line/model-no-compat` 的接手者抓到。
  run "clippy[$crate]" cargo clippy -p "$crate" --all-targets ${extra[@]+"${extra[@]}"} -- -D warnings
  step "crate $crate: test ${extra[*]:-}"
  run "test[$crate]" cargo test -p "$crate" ${extra[@]+"${extra[@]}"}
}

case "$MODE" in
  light)
    gate_fmt
    gate_guards
    gate_docs
    gate_license_inventory
    ;;
  crate)
    [[ $# -ge 1 ]] || fail "用法: run-gates.sh crate <crate-name> [更多 crate...]"
    gate_fmt
    gate_guards
    gate_docs
    gate_license_inventory
    for crate in "$@"; do gate_crate "$crate"; done
    ;;
  deny)
    gate_deny
    ;;
  full)
    if [[ -z "${CI:-}" && -z "${YEBAN_ALLOW_HEAVY:-}" ]]; then
      fail "full 档位只能跑在 CI 上。本机请用 light / crate <name>, 或显式 YEBAN_ALLOW_HEAVY=1。"
    fi
    gate_fmt
    gate_guards
    gate_docs
    gate_license_inventory
    gate_schemas
    step "cargo clippy --workspace --all-targets -- -D warnings"
    run "clippy[workspace]" cargo clippy --workspace --all-targets -- -D warnings
    step "cargo test --workspace --all-targets"
    run "test[workspace]" cargo test --workspace --all-targets
    gate_deny
    ;;
  *)
    fail "未知档位 '$MODE' (可选: light | crate <name> | full)"
    ;;
esac

printf '\n\033[32m门禁通过\033[0m (mode=%s)\n' "$MODE"
