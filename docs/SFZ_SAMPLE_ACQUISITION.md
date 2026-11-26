# SFZ 采样外取一步到位（登记式素材）

**为什么仓库里没有 `.wav`**：`assets/samples/manifest.json` 的 `policy` 是**登记式
（registry-only）**——官方仓库不随包分发任何采样字节。全部素材 20 594 个登记文件、
9 844 170 377 字节（总上游 9.371 GiB，含排除项），超过 GitHub 单次 push 上限，也不该让
每次 clone 背素材。清单里 30 件乐器只登记**指针 + 摘要**（`repo` + `pin` 或 `archive.url`
+ 每条文件的 `sha256`）。

**由此产生的静默缺口（G4）**：直接 `yeban_load_sfz` 一份采样未落地的 SFZ，`status`
仍是 `success`（文本确实解析、registry 确实注册了），但 `data.warnings[]` 会点名
`sfzSamplesUnreadable`；绑到采样器轨后 `yeban_render_master` **整轨数字静音**——
采样器路径没有正弦回退。要出声，必须先按下面的步骤把字节取回来。

## 1. 取一个乐器（git 溯源形态，28 款）

清单字段：`instruments[].repo` + `pin`（提交 SHA）+ `prefix`；每个文件的落点 =
`items[].relative_path`（= `assets/samples/<prefix>/<仓库内路径>`，上游树是**整体镜像**，
SFZ 的 `sample=` 相对路径因此天然对得上）。

```bash
id=aliexpress-erhu                      # instruments[].id
# 从清单读 repo / pin / prefix
read -r repo pin prefix <<<"$(python3 -c "
import json; m=json.load(open('assets/samples/manifest.json'))
i=[x for x in m['instruments'] if x['id']=='$id'][0]
print(i['repo'], i['pin'], i['prefix'])")"
git clone --bare --single-branch "https://github.com/$repo" "/tmp/$id.git"
git --git-dir="/tmp/$id.git" archive "$pin" | tar -xc "assets/samples/$prefix"
```

## 2. 取一个乐器（archive 溯源形态，4 款：2 个 FreePats 风琴 + 2 个 Karoryfer）

这些条目没有可直接按文件取回的 repo+pin（或压缩包成员路径 ≠ 镜像路径），走
`instruments[].archive = {asset, url, bytes, sha256}`：先下载**整个压缩包**并按登记的
`sha256`/字节校验，再解出成员放进 `relative_root`。生成器当年的成员映射原文见
`docs/ledger/samples-attribution-notes.md` §转换规则与附录。

## 3. 校验（无论哪种形态，落地后都必须做）

逐文件比 sha256（判据来自 `items[]`，⛔ 不接受"看起来能用"）：

```bash
python3 - assets/samples/manifest.json <instrument-id> <<'EOF'
import hashlib, json, pathlib, sys
m = json.load(open(sys.argv[1])); tgt = sys.argv[2]
ok = bad = miss = 0
for it in m["items"]:
    if it["instrument"] != tgt:
        continue
    p = pathlib.Path(it["relative_path"])
    if not p.exists():
        miss += 1; continue
    if hashlib.sha256(p.read_bytes()).hexdigest() == it["sha256"]:
        ok += 1
    else:
        bad += 1
print(f"{tgt}: ok={ok} 摘要不符={bad} 仍缺失={miss}")
EOF
```

三条判据：`仍缺失=0`、`摘要不符=0`，然后**重跑** `yeban_load_sfz`——它每次装载都真的
重读磁盘（`sampleFailures` / `sampleSources` 逐字反映现状），警告消失才算取到位。

## 4. 未完成的事（如实登记，不冒充已交付）

- **拉取/校验脚本未提升到 `scripts/dev/`**：见 `docs/ledger/samples-attribution-notes.md`
  §needs **N3**（该工作线禁改 `scripts/**`）。本文的命令是可复制的替代形态。
- **`optional=true` 门禁不因缺文件报错**：这是登记式策略的**设计**，不是漏洞；
  它也正是"载入成功但静音"曾经无人察觉的土壤——现在由 `data.warnings[]` 补上点名。
- **30 款 ≠ 规范要求的 323 款**（差 293，`counts_note`），开箱音色的覆盖面是另一条缺口。

## 5. 采样根边界与缺样本告警口径（T-FIX-266）

`resolve_sample_path` 的沙箱根是 **instrument 根**，不是 SFZ 文件所在的目录：

- 当 SFZ 在 `<instrument>/Programs/*.sfz` 时，根取 `Programs` 的父目录；其它布局
  仍以 SFZ 所在目录为根，**不扩大**。
- sample 里的 `..` 先在词法层归约：先用掉「从 SFZ 目录回到根」的额度（Programs
  布局为 1），再弹已归约的组件；**弹空即 `SampleEscape`**。例如
  `..\Samples\x.wav` 折成根内的 `Samples/x.wav`，而 `..\..\etc\passwd` 在第二次
  弹栈时被拒。
- `yeban_sfz::instrument::validate_sample_path` 一字未改——它仍然拒绝裸 `..`；
  这里只是把归约后的逻辑路径交给它，让它照旧拦绝对路径 / 盘符相对 / NUL。
  symlink 逃逸仍由 `canonicalize` + `starts_with(root_canonical)` 兜底。
  ⛔ 不是把沙箱放宽成"任意上级目录"。
- **外取落点不变**：仍按 `assets/samples/<prefix>/<仓库内路径>` 整体镜像安装，
  `Programs/` 与 `Samples/` 保持上游相对关系（见第 1 节）；缺样本时重新取字节、
  重跑装载即可。

`apply_preset` 的 `samplerBinding` 告警按 ADR-0024 的 sample 告警口径聚合：
同一缺失样本只报一次，一条告警带 `count`（去重后的个数）、最多 5 条 `examples`、
`hint` 与 `blocking: false`，不再按 region 逐条平铺。
