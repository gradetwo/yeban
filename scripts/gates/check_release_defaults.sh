#!/usr/bin/env bash
# MUST-GATE-009：**发行物层面**的"危险能力默认关"断言。
#
# 规范要求（`docs/ledger/gate-status.md` 的 MUST-GATE-009 一行）：进程内已有 112 条判据（默认关 /
# 只绑环回 / 0600 / `ui:inject` 先于 token 硬禁），**缺的是"发行物层面"的断言** —— 也就是：
# **默认构建出来的那个二进制**，运行时到底听不听话？
#
# 本脚本不问源码（源码层的 `default = []` 只证明清单写法），而是**问产物**，分两半：
#   ① **不带开关**跑 ⇒ 必须**不起监听**（走 stdio、读到 EOF 就退出）；
#   ② **带 `--enable-mcp-http`** 跑 ⇒ 两种产物形态都合法，但各自必须自证：
#      (a) 本次构建**没编** `mcp-http` ⇒ 必须**非零退出且点名 feature**
#          （证明拒绝是因为"没编进来"，不是别的原因）；
#      (b) 本次构建**编了** `mcp-http` ⇒ 服务必须**真起来并能应答**：只绑 `127.0.0.1`、
#          无令牌请求必须被 401 挡下、带令牌的 `tools/list` 必须应答 HTTP 200。
#
# ## 为什么改成"有界探针"（2026-11-16 修订，与 2026-11-12 裁决的对应关系）
#
# 第一版写于 2026-11-12 裁决（ADR-0072：`yeban-app` 的 `in-process-mcp` 默认开）之前，
# 用 `"$bin" --enable-mcp-http | head -c 400` 同步取输出 —— 它假设这个开关永远只会
# 换来一句拒绝。裁决之后，`release.yml` 用**一次** `cargo build -p yeban-app -p yeban-mcp`
# 构建发行物，cargo 的 feature 统一使这次构建出的 `yeban-mcp` 产物**也**带上了
# `mcp-http`：对这个产物，`--enable-mcp-http` 会真的起环回服务并常驻，管道永远等不到
# EOF ⇒ **死锁**（Release dry_run run 43826333760 实证：build 腿卡死直至被取消，
# runner 日志里留有被杀的孤儿 `yeban-mcp` 与 `head`）。
#
# ADR-0072 D4 说本脚本"不受本裁决影响"，指的是**单独** `cargo build -p yeban-mcp`
# 的产物（手动档 inventory 的路径）仍走 (a) 拒绝路径，本脚本对它一字未变；发行构建的
# 统一产物走 (b)。判据意图与裁决原文一致：**编译进来不等于在监听 —— 不带开关
# （且未设 `YEBAN_MCP_HTTP=1`）不许监听；显式开了，就必须只在环回上真服务。**
# 本脚本对 `yeban-mcp` 产物设的探针正是这句话的两半，探针语义：
# 后台启动产物 → 限时轮询（自行退出 / 打印监听地址 / 到期，三选一必达）→ 断言 →
# **必杀**（EXIT/INT/TERM trap 兜底，失败路径也不留孤儿进程）。任何一侧都有界，
# 不再有等不到 EOF 的管道。
#
# 纪律：**工具/产物缺失一律记 `unknown`（exit 2），绝不记 FAIL**（假红会让门禁变成噪音）。
set -uo pipefail
repo_root="$(git rev-parse --show-toplevel)" || exit 2
cd "$repo_root"
if [ -f scripts/dev/local-env.sh ]; then . scripts/dev/local-env.sh >/dev/null 2>&1 || true; fi
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env" >/dev/null 2>&1 || true

bin=""
for cand in target/debug/yeban-mcp target/release/yeban-mcp; do
  [ -x "$cand" ] && bin="$cand" && break
done
if [ -z "$bin" ]; then
  if command -v cargo >/dev/null 2>&1; then
    printf '[run] 未找到已构建的 yeban-mcp，尝试 cargo build -p yeban-mcp（默认 feature）\n'
    cargo build -p yeban-mcp >/dev/null 2>&1 && [ -x target/debug/yeban-mcp ] && bin=target/debug/yeban-mcp
  fi
fi
if [ -z "$bin" ]; then
  printf '[unknown] 没有可用的 yeban-mcp 产物（或 cargo 不可用）—— 本环境无法判定 MUST-GATE-009 的产物级断言\n'
  exit 2
fi
printf '[info] 产物: %s\n' "$bin"

fail=0

# ---------------------------------------------------------------------------
# 有界探针基础设施：全局 probe_pid / probe_dir 由 trap 兜底清理。
# ---------------------------------------------------------------------------
probe_pid=""
probe_dir=""
probe_rc=""

kill_probe() {
  [ -n "$probe_pid" ] || return 0
  kill "$probe_pid" 2>/dev/null || true
  local i=0
  while [ "$i" -lt 20 ]; do
    kill -0 "$probe_pid" 2>/dev/null || break
    sleep 0.1
    i=$((i + 1))
  done
  if kill -0 "$probe_pid" 2>/dev/null; then
    kill -9 "$probe_pid" 2>/dev/null || true
  fi
  wait "$probe_pid" 2>/dev/null || true
  probe_pid=""
}

cleanup_probe() {
  kill_probe
  if [ -n "$probe_dir" ]; then
    rm -rf "$probe_dir"
    probe_dir=""
  fi
}
trap cleanup_probe EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# start_probe <输出文件> [产物参数...]：后台启动产物，stdin 接 /dev/null，
# 并抹掉 YEBAN_MCP_HTTP —— 探针要量的是"开关本身"的效果，不许环境变量替它开。
start_probe() {
  local out_file="$1"
  shift
  env -u YEBAN_MCP_HTTP "$bin" "$@" >"$out_file" 2>&1 </dev/null &
  probe_pid=$!
}

# probe_exited：探针进程已自行退出时返回 0，并把退出码放进 probe_rc。
probe_exited() {
  if [ -n "$probe_pid" ] && ! kill -0 "$probe_pid" 2>/dev/null; then
    wait "$probe_pid" 2>/dev/null
    probe_rc=$?
    probe_pid=""
    return 0
  fi
  return 1
}

# http_status <url> <token|空>：发一次 JSON-RPC tools/list，**只**打印 HTTP 状态码；
# 传输层失败打印空串。优先 curl；没有 curl 时用 bash 的 /dev/tcp 在子壳里兜底
# （子壳隔离 redirection 失败，避免 exec 失败把整个脚本带走）。
http_status() {
  local url="$1" token="$2"
  if command -v curl >/dev/null 2>&1; then
    local code
    if [ -n "$token" ]; then
      code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST "$url" \
        -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' 2>/dev/null || true)"
    else
      code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -X POST "$url" \
        -H 'Content-Type: application/json' \
        --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' 2>/dev/null || true)"
    fi
    [ "$code" = "000" ] && code=""
    printf '%s' "$code"
    return 0
  fi
  (
    hostport="${url#http://}"
    hostport="${hostport%%/*}"
    host="${hostport%%:*}"
    rest="${hostport#*:}"
    port="${rest%%/*}"
    path="/${url#http://*/}"
    body='{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
    exec 9<>"/dev/tcp/$host/$port" || exit 0
    if [ -n "$token" ]; then
      printf 'POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nAuthorization: Bearer %s\r\nContent-Length: %s\r\n\r\n%s' \
        "$path" "$host" "$token" "${#body}" "$body" >&9
    else
      printf 'POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\n\r\n%s' \
        "$path" "$host" "${#body}" "$body" >&9
    fi
    line=""
    IFS= read -r -t 10 line <&9 || true
    exec 9>&-
    exec 9<&-
    printf '%s' "$line" | awk '{print $2}'
  )
}

probe_dir="$(mktemp -d 2>/dev/null || true)"
if [ -z "$probe_dir" ] || [ ! -d "$probe_dir" ]; then
  printf '[unknown] mktemp -d 失败 —— 本环境无法布置探针临时目录\n'
  exit 2
fi

# ---------------------------------------------------------------------------
# 探针 ②（带开关）先行 —— 沿用第一版的检查顺序。
#
# ⚠ 历史教训（保留）：第一版这条判据曾经**空转**：传了 `--port 39876`，而该二进制
# **没有 `--port`** ⇒ 它以 rc=2 "未知参数" 退出，却被当成"拒绝了开关"的证据。
# **任何"因为别的原因失败"都能骗过这种写法。** ⇒ 至今只传**真实存在的**
# `--enable-mcp-http`（见 `--help`），且 (a) 路径要求：非零退出 **且** 输出点名
# feature/编译（证明它是因为"没编进来"而拒绝，不是别的原因）。
# （`--token-file` 同为真实存在的参数；探针把令牌落进临时目录，不碰 ~/.yeban。）
# ---------------------------------------------------------------------------
a_out="$probe_dir/with-flag.out"
start_probe "$a_out" --enable-mcp-http --token-file "$probe_dir/session.token"
a_state="timeout"
i=0
while [ "$i" -lt 200 ]; do
  if grep -q 'http://' "$a_out" 2>/dev/null; then
    a_state="serving"
    break
  fi
  if probe_exited; then
    a_state="exited"
    break
  fi
  sleep 0.1
  i=$((i + 1))
done

if [ "$a_state" = "exited" ]; then
  # (a) 产物自行退出 ⇒ 第一版的拒绝断言**原样**适用（措辞与判定顺序一字未动，
  #     只是输出改从落盘文件读，不再走会死锁的管道）。
  out="$(cat "$a_out")"
  rc="$probe_rc"
  if [ "$rc" -eq 0 ]; then
    printf 'FAIL 默认产物接受了 --enable-mcp-http（rc=0）—— 默认构建里可能真的链进了网络监听代码\n' >&2
    fail=1
  elif printf '%s' "$out" | grep -qiE 'mcp-http|feature|未编译|not compiled|需要'; then
    printf '[ok] 默认产物拒绝 --enable-mcp-http 且点名 feature（rc=%s）: %s\n' "$rc" "$(printf '%s' "$out" | head -1)"
  elif printf '%s' "$out" | grep -qiE 'Operation not permitted|Permission denied|os error|令牌文件 I/O|只读文件系统'; then
    # ⚠ 环境级失败**不是**缺陷：例如沙箱不允许写令牌文件 ⇒ 二进制在走到开关逻辑**之前**就退出了。
    # 这与"产物把监听代码链进来了"是两件事 ⇒ 记 **unknown（exit 2）**，绝不记 FAIL。
    printf '[unknown] 产物因**环境**原因提前退出（未能走到开关判定）: %s\n' "$(printf '%s' "$out" | head -1)"
    exit 2
  else
    printf 'FAIL 默认产物拒绝了该开关, 但**没有**点名 feature —— 无法证明拒绝是因为"没编进来"（可能是别的原因）\n' >&2
    printf '      输出: %s\n' "$(printf '%s' "$out" | head -c 400)" >&2
    fail=1
  fi
elif [ "$a_state" = "serving" ]; then
  # (b) 产物打印了监听地址 ⇒ 它是带 feature 的构建（发行统一构建正是这一路）。
  #     旧版在这里只 grep 输出里有没有 `0.0.0.0`；现在直接按它自报的地址验三件事：
  #     地址必须是环回、无令牌必须 401、带令牌必须 200 —— 比旧断言更强，不更弱。
  url="$(grep -oE 'http://[^ ]+' "$a_out" 2>/dev/null | head -1)"
  printf '[info] 带该 feature 的产物，显式开关下打印监听地址: %s\n' "$url"
  hostport="${url#http://}"
  hostport="${hostport%%/*}"
  host="${hostport%%:*}"
  port="${hostport#*:}"
  if [ "$host" != "127.0.0.1" ]; then
    printf 'FAIL 监听地址不是环回（%s）—— MUST-GATE-009 只许绑 127.0.0.1\n' "$url" >&2
    fail=1
  elif [ ! -s "$probe_dir/session.token" ]; then
    printf 'FAIL 服务已打印监听地址，但令牌文件未生成或为空: %s\n' "$probe_dir/session.token" >&2
    fail=1
  else
    token="$(tr -d '[:space:]' <"$probe_dir/session.token")"
    code_noauth="$(http_status "$url" "")"
    code_auth="$(http_status "$url" "$token")"
    if [ "$code_noauth" != "401" ]; then
      printf 'FAIL 无令牌请求未被 401 挡下（实得 %s）—— 令牌闸门在产物级未生效\n' "${code_noauth:-无应答}" >&2
      fail=1
    fi
    if [ "$code_auth" != "200" ]; then
      printf 'FAIL 带令牌的 tools/list 未得到 HTTP 200（实得 %s）—— 服务起来了却不应答\n' "${code_auth:-无应答}" >&2
      fail=1
    fi
    if [ "$fail" -eq 0 ]; then
      printf '[ok] 带 feature 的产物: --enable-mcp-http 在 127.0.0.1:%s 真启动; 无令牌被 401 挡下; 带令牌 tools/list 应答 HTTP 200\n' "$port"
    fi
  fi
  kill_probe
else
  # 20 秒内既没退出、也没打印监听地址：无法证明它在做什么（静默挂死或静默监听）。
  # 这正是旧版死锁的形态 —— 现在到期即判红并杀掉，不再无限等。
  printf 'FAIL 带 --enable-mcp-http 启动 20 秒内既未退出拒绝、也未打印监听地址 —— 无法证明它在做什么（探针已将其杀掉，不留孤儿）\n' >&2
  printf '      输出: %s\n' "$(head -c 400 "$a_out" 2>/dev/null)" >&2
  fail=1
  kill_probe
fi

# ---------------------------------------------------------------------------
# 探针 ①（不带开关）：编译进来不等于在监听。
# 本产物在无开关时只有 stdio 一条路：stdin 已接 /dev/null（EOF），应当立即退出。
# 它唯一"不退出"的代码路径是 HTTP 服务循环 ⇒ 到期仍活着 = 未挂开关却在监听。
# ---------------------------------------------------------------------------
b_out="$probe_dir/no-flag.out"
start_probe "$b_out"
b_state="timeout"
i=0
while [ "$i" -lt 100 ]; do
  if grep -q 'http://' "$b_out" 2>/dev/null; then
    b_state="announced"
    break
  fi
  if probe_exited; then
    b_state="exited"
    break
  fi
  sleep 0.1
  i=$((i + 1))
done

if [ "$b_state" = "exited" ]; then
  out="$(cat "$b_out")"
  rc="$probe_rc"
  if [ "$rc" -eq 0 ]; then
    printf '[ok] 不带开关: 产物走 stdio, 读到 EOF 即退出（rc=0）, 未起监听\n'
  elif printf '%s' "$out" | grep -qiE 'Operation not permitted|Permission denied|os error|令牌文件 I/O|只读文件系统'; then
    printf '[unknown] 产物因**环境**原因提前退出（未能走到开关判定）: %s\n' "$(printf '%s' "$out" | head -1)"
    exit 2
  else
    printf 'FAIL 不带开关时产物异常退出（rc=%s）: %s\n' "$rc" "$(printf '%s' "$out" | head -1)" >&2
    fail=1
  fi
elif [ "$b_state" = "announced" ]; then
  printf 'FAIL 不带开关时产物打印了监听地址 —— 未显式开启却在监听: %s\n' "$(grep -oE 'http://[^ ]+' "$b_out" 2>/dev/null | head -1)" >&2
  fail=1
  kill_probe
else
  printf 'FAIL 不带开关时产物 10 秒内未退出 —— stdio 在 stdin EOF 下应当立即结束; 本产物唯一不退出的路径是 HTTP 服务循环 ⇒ 判定为未挂开关却在监听（探针已将其杀掉，不留孤儿）\n' >&2
  fail=1
  kill_probe
fi

[ "$fail" -eq 0 ] && { printf 'check_release_defaults: 通过（MUST-GATE-009 的产物级部分）\n'; exit 0; }
exit 1
