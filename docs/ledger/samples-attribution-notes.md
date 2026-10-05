# 工作线 `samples-attribution` 台账 (MUST-GATE-014 素材登记)

> 工作树: `yeban/.worktrees/samples-attribution`(分支 `line/samples-attribution`)
> 裁决依据: `ADR-0001` **D54** —— `MUST-GATE-014` 的素材**先暂时复用 `groove` 之前的选择 / 登记 / 计算**。
> 规范原文: `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:390`。

## 0. 结论摘要 (TL;DR)

| 项 | 事实 |
| :--- | :--- |
| 规范目标 | **323 款**原声乐器指纹与 `assets/samples/ATTRIBUTION.md` 逐条吻合 |
| 本次实际登记 | **30 款**(27 CC0 + 3 CC-BY), **20 594 个文件**, 9 844 170 377 字节(9.168 GiB) |
| **与 323 的差额** | **323 − 30 = 293 款未登记**(一行看得见的账, §8) |
| 过滤掉 | **3 款**(CC-BY-NC-SA / CC-Sampling-Plus / Unlicense 待裁决), 911 个文件 / 207.6 MiB |
| 入库形态 | **登记式(registry-only), 零字节入库**: 清单 8.5 MB, 素材 9.371 GiB **不复制进仓库** |
| `--repo-assets` 门禁 | **绿**(EXIT=0) —— 但它同时如实打印 `0/20594 …(另有 20594 项 optional 资产未随仓库分发)` |
| 真字节校验 | **84 个文件的真实字节与清单一致, 0 失败**(含全局最小 0 字节与全局最大 7 836 692 字节) |
| 发现的缺口 | 门禁**不读** `license`, 许可白名单**没有**机械保护(注入 CC-BY-NC-SA 后仍绿, 实测) |

**没有**做的事: 没有把素材字节复制进仓库; 没有用夹具/占位文件凑数; 没有把 30 写成 323;
没有为了让门禁变绿而只登记 sha256 却从不校验真实字节(§7 是真下载校验)。

## 1. 来源与一手事实 (Source of Truth)

- 源: `/Users/crow/work/music/groove/public/samples/manifest.json`
  - 字节数 **5 216 706**, SHA-256 `0a77ad086436eef2b1a5fea20bc7e5ca1e511d6f8e8dfce71ab837e877bfd469`
  - 结构 `{version: 1, entries: [33]}`
  - 上游仓库: `apple2011:music/groove`,HEAD `606087d1`
- **关键一手事实(实测, 与任务书里"可能数百 MB～GB"的量级不符)**:
  - `groove/public/samples/` 里**只有一个 `manifest.json`**;
  - `find groove/public/samples -type f ! -name manifest.json | wc -l` = **0**;
  - `du -sh groove/public/samples` = **5.0M**。
  - ⇒ **9.371 GiB 的素材字节在本机根本不存在**。`groove` 只做了登记, 没有落盘(或已清理)。
    这直接决定了本次只能做**登记**, 也决定了"校验真实字节"必须**去上游拉**(§7)。
- 33 个条目的字段: `id`/`name`/`licence`/`prefix`/`repo`/`pin`/`sfz`/`needs`(SFZ opcode 白名单)/
  `files:[{path,bytes,sha256,durationSeconds?}]`/`sourceUrl`/`category`/`mirroredAt`/`archive?`
- 文件总数 **21 505**(全部带 sha256), 合计 **10 061 840 365 字节(9.371 GiB)**。
- 尺寸分布: 最大单文件 7 836 692 字节, 最小 0 字节(一个 `.txt`); >1 MB 的有 2 545 个; >10 MB 的 **0** 个。
- 扩展名: wav 11 994 / flac 7 722 / sfz 1 494 / txt 174 / ariax 52 / (无) 23 / ogg 18 / md 12 / mp3 6 / xml 6 / png 4。
- 两种溯源形态(都必须同等对待):
  - **git**: `repo` + `pin`(28 款), 可 `raw.githubusercontent.com/<repo>/<pin>/<path>` 单文件拉取;
  - **archive**: `archive{asset,url,bytes,sha256}`(4 款)= 2 款 FreePats 风琴(`.tar.xz`, 无 git repo)
    + 2 款 Karoryfer Meatbass / Emilyguitar(`.zip`, **同时**有 repo+pin)。
- 2 个条目没有 `repo`/`pin`(`freepats-drawbar-organ`、`freepats-percussive-organ`), 它们走 `archive` 形态
  —— 这是**另一种合法溯源**, 不是缺字段。

## 2. 转换规则 (groove → yeban)

以**代码为准**: 读 `scripts/gates/validate_schemas.py` 与 `schemas/assets.manifest.schema.json` 后确定形状。

| 决策 | 依据 |
| :--- | :--- |
| 文件名 `assets/samples/manifest.json` | 门禁按 `path.name.lower() == "manifest.json"` 发现清单(小写更稳) |
| **一文件一条 `items[]`** | schema 的 `item` 必填 `id/name/relative_path/sha256/license/commercial_usable`, 语义是"一个文件的登记"; 仓库既有 `assets/brand/manifest.json` 也是**一文件一条**。故 20 594 条 |
| `relative_path` = `assets/samples/<prefix>/<path>` | schema 的 description 明确"相对**仓库根**", 与根清单同口径; 门禁用它 `REPO / rel_path` 再重算 sha256 |
| `id` = `<prefix>/<path>` | 在清单内唯一(实测 20 594 个 id 全唯一, `relative_path` 亦全唯一); 与 `relative_path` 差一个常量前缀 |
| **`optional: true`(全部条目)** | schema: `optional` = "官方仓库**不随包分发**该资产…这是登记义务, 不是打包义务"; 门禁对 `optional` 缺失文件不报错, 只计数 |
| `size_bytes` 逐条登记 | 门禁会拿它与磁盘 `stat().st_size` 对账 |
| 乐器级元数据放顶层 `instruments[]` | 含 `prefix`/`license`/`repo`/`pin`/`source_url`/`sfz`/`needs`/`file_count`/`total_bytes`/`fingerprint`。**注意 `id` 与 `prefix` 可能不同**(实测 `virtuosity-drums-basic` 的 prefix 是 `virtuosity-drums`), 所以两个字段都要存 —— 这是本次实际踩到的坑 |
| 非白名单条目只进 `excluded[]` | **绝不进 `items[]`**。`items[]` 是"允许入库"的唯一载体 |
| 顶层 `licence_whitelist` + `licence_whitelist_enforced_by_gate: false` | 把"白名单没有机械保护"这件事写在机器可读的位置, 而不是只写在文档里 |
| 确定性 | 输入同一份 groove manifest ⇒ 输出**逐字节相同**。所有列表显式排序, 字段顺序由构造顺序固定 |

许可映射(上游 literal → SPDX):

| 上游 `licence` | 映射为 | 处置 |
| :--- | :--- | :--- |
| `CC0` (27) | `CC0-1.0` | 登记, `commercial_usable: true`, `attribution_required: false` |
| `CC-BY` (3) | 逐乐器核实版本(见下) | 登记, `commercial_usable: true`, `attribution_required: true` |
| `CC-BY-NC-SA` (1) | `CC-BY-NC-SA` | **过滤** |
| `CC-Sampling-Plus` (1) | `CC-Sampling-Plus-1.0` | **过滤** |
| `Unlicense` (1) | `Unlicense` | **过滤 + 待裁决**(§needs N1) |

`CC-BY` 无法从字面区分版本, 故**逐个查上游核实**(不是猜):

| 乐器 | 核实结果 | 依据 |
| :--- | :--- | :--- |
| Salamander Grand Piano | **CC-BY-3.0**, Alexander Holm | 仓库 README §License: "Creative Commons Attribution 3.0 Unported License" |
| MTG Solo Saxophones | **CC-BY-4.0**, MTG (Universitat Pompeu Fabra), SFZ 转换 kinwie | GitHub License API `spdx_id: CC-BY-4.0`; README 指明样本来自 MTG freesound packs |
| Ixox Flute | **CC-BY-4.0**, Xavier Hosxe | GitHub License API `spdx_id: CC-BY-4.0`; README: "by Xavier Hosxe" |

⇒ 由此发现: 根清单 `assets/manifest.json` 的 `allowed_licenses` 原先**只有 `CC-BY-4.0`, 漏了 `CC-BY-3.0`**,
而规范原文写的是「CC0/CC-BY/MIT」(未限定版本)。本次已把 `CC-BY-3.0` 补进白名单, 并在根清单 `note` 里写明原因。

**乐器指纹口径**(`MUST-GATE-014` 要求"每款乐器的指纹"), 定义并登记在清单每条 `instruments[]` 的
`fingerprint` + `fingerprint_recipe` 字段, 可独立复算:

```text
sha256( concat( path + '\n' + sha256 + '\n' for each file, sorted by path ) )
```

**输出体积的一个硬约束(必须记下来)**: `scripts/guards/policy_check.py` 的 **G06** 会拒绝**任何**
>10 MiB 的文件, 而 `LARGE_FILE_ALLOWLIST` 在 `scripts/`(本工作线**禁改**)里且是空的。
用 `indent=2` 全量缩进序列化时清单是 **10 648 338 字节 ⇒ G06 判红**(已实测, §6 附加判据);
因此 `items[]` 改为**一条一行**的紧凑序列化 ⇒ **8 903 874 字节**, G06 绿。
副产品: 某个文件的 sha256 变了只产生**一行** diff, 而不是 12 行。

## 3. 白名单过滤结果 (逐条, 33 → 30)

上游 33 条, 登记 **30** 条, 过滤 **3** 条。

| # | 乐器 id | 名称 | 上游 `licence` | 文件数 | 字节 | 处置 |
| ---: | :--- | :--- | :--- | ---: | ---: | :--- |
| 1 | `aliexpress-erhu` | AliExpress erhu | `CC0` | 191 | 92826029 | **登记** `CC0-1.0` |
| 2 | `body-percussion` | Body Percussion | `CC0` | 233 | 61733852 | **登记** `CC0-1.0` |
| 3 | `cithara-barbarica` | Cithara barbarica | `CC0` | 275 | 239034087 | **登记** `CC0-1.0` |
| 4 | `discord-gm-sitar` | Discord SFZ GM Bank — Sitar | `CC0` | 49 | 10149165 | **登记** `CC0-1.0` |
| 5 | `dsmolken-double-bass` | D. Smolken Rübner double bass | `CC0` | 406 | 288893197 | **登记** `CC0-1.0` |
| 6 | `freepats-button-accordion-hn` | FreePats Button Accordion HN | `CC0` | 37 | 4470533 | **登记** `CC0-1.0` |
| 7 | `freepats-drawbar-organ` | FreePats Drawbar Organ Emulation | `CC0` | 18 | 6505808 | **登记** `CC0-1.0` |
| 8 | `freepats-electric-bass-yr` | FreePats Electric Bass Guitar YR | `CC0` | 29 | 6277261 | **登记** `CC0-1.0` |
| 9 | `freepats-fsbs-dist2` | FreePats FSBS Electric Guitar Distorted #2 | `CC0` | 125 | 136921363 | **登记** `CC0-1.0` |
| 10 | `freepats-percussive-organ` | FreePats Percussive Organ Emulation | `CC0` | 34 | 14548534 | **登记** `CC0-1.0` |
| 11 | `freepats-spanish-classical-guitar` | FreePats Spanish Classical Guitar | `CC0` | 51 | 5298000 | **登记** `CC0-1.0` |
| 12 | `ganjo` | Ganjo | `CC0` | 65 | 24493786 | **登记** `CC0-1.0` |
| 13 | `hungarian-zither` | Hungarian zither | `CC0` | 219 | 178322902 | **登记** `CC0-1.0` |
| 14 | `ixox-flute` | Ixox Flute | `CC-BY` | 161 | 10215135 | **登记** `CC-BY-4.0` |
| 15 | `jlearman-jrhodes3c` | jRhodes3c — 1977 Rhodes Mark I Stage 73 | `CC-BY-NC-SA` | 136 | 17566470 | **过滤**(非商用) |
| 16 | `jlearman-steel-drum` | jSteelDrum — C steel drum | `Unlicense` | 372 | 38541639 | **过滤 + 待裁决** |
| 17 | `karoryfer-272-merry-orks` | 272 Merry Orks | `CC0` | 277 | 45627769 | **登记** `CC0-1.0` |
| 18 | `karoryfer-bear-sax` | Bear Sax | `CC0` | 814 | 143854044 | **登记** `CC0-1.0` |
| 19 | `karoryfer-big-rusty-drums` | Big Rusty Drums | `CC0` | 4814 | 706838139 | **登记** `CC0-1.0` |
| 20 | `karoryfer-bigcat-cello` | Karoryfer × bigcat cello | `CC0` | 520 | 141812275 | **登记** `CC0-1.0` |
| 21 | `karoryfer-black-and-blue-basses` | Karoryfer Black And Blue Basses | `CC0` | 2274 | 1124158166 | **登记** `CC0-1.0` |
| 22 | `karoryfer-cowsynth` | Cowsynth | `CC0` | 59 | 14832493 | **登记** `CC0-1.0` |
| 23 | `karoryfer-emilyguitar` | Karoryfer Emilyguitar | `CC0` | 331 | 125538629 | **登记** `CC0-1.0` |
| 24 | `karoryfer-meatbass` | Karoryfer Meatbass | `CC0` | 587 | 296405063 | **登记** `CC0-1.0` |
| 25 | `karoryfer-pastabass` | Karoryfer Pastabass (linguine) | `CC0` | 207 | 125954822 | **登记** `CC0-1.0` |
| 26 | `karoryfer-squidpipes` | Squidpipes | `CC0` | 437 | 51873578 | **登记** `CC0-1.0` |
| 27 | `karoryfer-string-cyborgs` | String Cyborgs | `CC0` | 304 | 74501792 | **登记** `CC0-1.0` |
| 28 | `mtg-solo-sax` | MTG Solo Saxophones | `CC-BY` | 784 | 110931713 | **登记** `CC-BY-4.0` |
| 29 | `salamander-grand` | Salamander Grand Piano | `CC-BY` | 667 | 748451231 | **登记** `CC-BY-3.0` |
| 30 | `sonatina-brass` | Sonatina Symphonic Orchestra — Brass | `CC-Sampling-Plus` | 403 | 161561879 | **过滤**(不在白名单) |
| 31 | `vcsl` | VCSL — Versilian Community Sample Library | `CC0` | 2651 | 2525491354 | **登记** `CC0-1.0` |
| 32 | `virtuosity-drums-basic` | Virtuosity Drums — Basic Kit | `CC0` | 2078 | 443075501 | **登记** `CC0-1.0` |
| 33 | `vsco2ce` | VSCO 2 CE — Versilian Studios Chamber Orchestra: Community Edition | `CC0` | 1897 | 2085134156 | **登记** `CC0-1.0` |

合计: 登记 **30** 款 / 过滤 **3** 款, 共 **33** 款。


## 4. 入库形态与体积实测 (Bundle vs Register)

**决策: 登记式(registry-only), 零字节入库。**

依据(实测数字, 不是估计):

| 量 | 数值 |
| :--- | :--- |
| 上游素材总量 | **10 061 840 365 字节 = 9.371 GiB**, 21 505 个文件 |
| 白名单内(本应登记的部分) | 9 844 170 377 字节 = **9.168 GiB**, 20 594 个文件 |
| 被过滤部分 | 217 669 988 字节 = **207.6 MiB**, 911 个文件 |
| **实际入库体积** `du -sh assets/samples` | **8.5 MB**(`manifest.json` 8.5 MB + `ATTRIBUTION.md` 16 KB) |
| 参考: `du -sh groove/public/samples` | 5.0 MB(只有 manifest.json) |

理由:

1. **GitHub 单次 push 上限 2 GB** —— 9.371 GiB 根本推不上去, 这不是审美偏好而是硬墙;
2. 每次 `git clone` 多背 9.4 GiB 不可接受(仓库当前文本规模是 MB 级);
3. 大文件入库会**永久**留在 git 历史里, 后续清理需要 `filter-repo` 重写历史;
4. 上游全部可公开拉取且**有 pin**, 仓库里存"指针 + 摘要"才是正确形态;
5. 素材字节**本机就不存在**(§1), 要"入库"就得先下载 9.371 GiB —— 收益与代价明显不成比例。

**代价与风险(如实登记, 不粉饰)**:

1. **门禁此刻一个字节都没校验**。原文输出:
   `[ok] assets/samples/manifest.json: 0/20594 条资产的 SHA-256 与磁盘一致(另有 20594 项 optional 资产未随仓库分发)`
   —— "绿"只代表**清单结构与登记口径成立**, 不代表素材可用;
2. **CI 不联网时无法校验任何采样字节**。本次的真实字节校验(§7)是一次性的**本机**动作, 不是 CI 的常规保护;
3. 缓解措施: 素材一旦落地(用户手动或 CI 拉取), 把对应条目的 `optional` 去掉,
   门禁立刻从"登记检查"升级为"20 594 个文件的真字节对账", **无需改一行门禁代码**。
   这就是把 `optional` 设计成逐条开关、而不是全局开关的原因。

## 5. `--repo-assets` 门禁: 本机原文输出

```text
$ python3 scripts/gates/validate_schemas.py --repo-assets
[ok] assets.manifest.schema.json: 合法 JSON Schema (AssetsManifest)
[ok] mcp-tools.schema.json: 合法 JSON Schema (YebanMcpTools)
[ok] ops.schema.json: 合法 JSON Schema (StampedOp)
[ok] project.schema.json: 合法 JSON Schema (YebanProjectDocument)
[ok] assets/brand/manifest.json: 21/21 条资产的 SHA-256 与磁盘一致
[ok] assets/manifest.json: 指针式清单, 3 个子清单均存在
[ok] assets/models/MANIFEST.json: 0/1 条资产的 SHA-256 与磁盘一致(另有 1 项 optional 资产未随仓库分发)
[ok] assets/samples/manifest.json: 0/20594 条资产的 SHA-256 与磁盘一致(另有 20594 项 optional 资产未随仓库分发)

契约校验通过 (4 份 schema)。
EXIT=0
```

以及机械红线守卫(14 条全绿, 含 G06 的 10 MiB 上限):

```text
$ python3 scripts/guards/policy_check.py
[ok  ] G06 [红线 9] 无 >10MB 未登记文件
… (G01–G14 全部 ok)
守卫全部通过 (14 条)。
EXIT=0
```

## 6. 判据与注入记录 (至少 6 条)

| # | 判据 | 做法 | 实测结果 |
| ---: | :--- | :--- | :--- |
| ① | `--repo-assets` 通过 | 真跑 | **EXIT=0**, 原文见 §5 |
| ② | 逐项重算 sha256 与清单一致 | 从**上游 pin** 真下载 32 个文件 + 2 个 tarball 的 52 个成员 + 2 个 zip 整包, 重算 sha256/大小 | **84 个文件全部一致, 0 失败**(含全局最小 0 字节与全局最大 7 836 692 字节), §7 |
| ③ | 未登记文件检查**有牙** | 放 `assets/samples/INJECT_kick.wav` ⇒ 跑门禁; 删掉 ⇒ 再跑 | 注入 **EXIT=1**, 报 `资产文件**未登记**在 assets/samples/manifest.json 里`; 删除后 **EXIT=0** |
| ④ | 清单出现非白名单许可 ⇒ 红 | 往 `items[]` 注入一条 `license: CC-BY-NC-SA` 的**结构合法**条目(其余字段齐备、`optional: true`, 隔离出"许可"这一个变量) | ⚠ **门禁仍然 EXIT=0(绿)** —— 见下方"负结果"。改用自己的白名单检查后 **EXIT=1** |
| ⑤ | 数量差一行可复核 | `len(items)`、登记乐器数、与 323 的差额都写进清单 `counts` 与 `ATTRIBUTION.md` §0 | `len(items)`=**20 594** == 实际登记文件数; 乐器 **30**; `323 − 30 = 293` |
| ⑥ | 确定性: 两次生成逐字节相同 | 同一输入连跑两次生成器, `cmp` 比对 + `shasum -a 256` | `cmp` **无差异**; 两次 sha256 均为 `39db1651ec6deb1b3831d8a69cac3155845a4b6e296be20ebebdbebe19fec84d` |
| 附加 | G06(>10 MiB)确实有牙 | 放入 `indent=2` 版清单(10 648 338 字节) | **G06 FAIL, EXIT=1**; 换回紧致版 **EXIT=0** |

### 判据 ④ 的**负结果**(必须如实报告, 不许粉饰成"通过")

任务书要求"清单里出现非白名单许可 ⇒ 红"。**实测这个判据在门禁里根本不存在**:

```text
$ grep -c "license" scripts/gates/validate_schemas.py
0
```

`validate_schemas.py` 只做三件事: 结构校验、逐项 SHA-256/大小对账、未登记文件扫描。
它**不读** `license`, 也**不读**根清单的 `allowed_licenses`(全仓库无任何代码读它们 —— 已 grep 全仓确认)。
注入一条 `CC-BY-NC-SA` 条目后:

```text
[ok] assets/samples/manifest.json: 0/20595 条资产的 SHA-256 与磁盘一致(另有 20595 项 optional 资产未随仓库分发)
契约校验通过 (4 份 schema)。
EXIT=0
```

⇒ 也就是说, **今天往 `items[]` 里塞一条非商用素材, 所有门禁依旧全绿**。
这是一个真实的合规缺口, 不是本工作线的产物。修它必须改 `scripts/`(集成者独占), 见 §needs N2。
在拿到那个修复之前, 本次用自建检查把它变为可见(注入即红):

```text
$ python3 - <<'PY'   # 白名单检查(真判据)
… RED jlearman-jrhodes3c/_jRhodes-both-looped.sfz: license=CC-BY-NC-SA 不在白名单 ['Apache-2.0','CC-BY-3.0','CC-BY-4.0','CC0-1.0','MIT']
EXIT=1
```

## 7. 真实字节校验 (为什么"只登记 sha256"不够)

门禁对 20 594 条 item **全部跳过**(磁盘上没有字节), 它打印 `0/20594`。
若到此为止, "登记"就是一句不可证伪的自我声明。所以本次**真的去上游拉了字节**:

选样规则(可复核): 全局最小 1 个 + 全局最大 1 个 + **每个 git 乐器**的 `.sfz` 入口文件
(全乐器覆盖) + 每种音频格式(wav/flac/ogg/mp3)最小的 1 个 = **32 个文件**;
再对 4 个 archive 溯源乐器校验整包 SHA-256, 其中 ≤32 MiB 的两个还**逐成员**校验。

```text
GLOBAL-MIN                         e3b0c44298fc   e3b0c44298fc            0          0  OK
GLOBAL-MAX                         6ffc24c2ac7f   6ffc24c2ac7f      7836692    7836692  OK
SFZ:aliexpress-erhu                b9e18076031d   b9e18076031d         2123       2123  OK
SFZ:body-percussion                60cbbe738b5d   60cbbe738b5d          229        229  OK
… (共 32 行, 全部 OK)
FMT-MIN:wav                        cd172562afcc   cd172562afcc         6988       6988  OK
FMT-MIN:flac                       c70ab2518ca7   c70ab2518ca7        11126      11126  OK
FMT-MIN:ogg                       6a2099fb221b   6a2099fb221b        82235      82235  OK
FMT-MIN:mp3                       5aaf152986bd   5aaf152986bd       167634     167634  OK

freepats-drawbar-organ: DrawbarOrganEmulation-SFZ-20190712.tar.xz 期望 e2da18b0a4d1… 实际 e2da18b0a4d1… OK
  成员文件校验: 18/18 与清单一致
freepats-percussive-organ: PercussiveOrganEmulation-SFZ-20190715.tar.xz 期望 c4841f2e7f35… 实际 c4841f2e7f35… OK
  成员文件校验: 34/34 与清单一致
karoryfer-emilyguitar: Karoryfer.Emilyguitar.v1.001.zip 期望 ffef3b289727… 实际 ffef3b289727… OK (103 484 350 字节)
  整包摘要一致 ⇒ 传递地确认了 331 个成员的全部字节
karoryfer-meatbass: Karoryfer.Meatbass.v1.001.zip 期望 bc053061d4f3… 实际 bc053061d4f3… OK (255 425 351 字节)
  整包摘要一致 ⇒ 传递地确认了 587 个成员的全部字节

结果: 84 个文件的真实字节与清单一致; 失败 0 条
EXIT=0
```

说明: 大 zip 只校验**整包** SHA-256 —— 整包摘要一致即**传递地**确认了每个成员的字节
(成员清单本来就出自同一个包), 因此没有再花 358 MB 去逐成员解包。
这份证据证明的是: **清单里的 sha256 是真实的上游字节摘要**, 不是编造或占位。

## 8. 与 323 的差额 (原文, 必须一行可见)

- 规范目标(`MUST-GATE-014`): **323 款**
- 本次登记: **30 款**
- **差额: 323 − 30 = 293 款未登记**

清单里机器可读地记着(`assets/samples/manifest.json` → `counts`):

```json
"instruments_spec_target": 323,
"instruments_registered": 30,
"instruments_excluded": 3,
"instruments_shortfall_vs_spec": 293,
```

以及 `counts_note`: 「规范 MUST-GATE-014 要求 323 款原声乐器; 本次**只**登记 30 款(27 CC0 + 3 CC-BY),
**差 293 款**。这不是 323, 也不得写成 323。」

根清单 `assets/manifest.json` 同时保留 `instruments_count: 323`(**规范目标**)与
`instruments_registered: 30` / `instruments_shortfall: 293`(实测), 避免 `323` 被误读成"已完成"。
`ATTRIBUTION.md` §0 同样先声明"登记了 30 款, 不是 323 款"。

## 9. 未实现项 (Explicitly NOT done)

1. **没有把 9.371 GiB 素材复制进仓库**(§4)。`items[]` 全部 `optional: true`, 磁盘上零字节。
2. **293 款乐器没有登记**, 因为没有素材来源 —— 这需要人类负责人裁决(`HD-31`), 不能由 Agent 发明。
3. **许可白名单仍未获得机械保护**(§6 负结果)。需要改 `scripts/`, 不在本工作线地盘内。
4. **`Unlicense` 的 jSteelDrum 未裁决**, 因此既不登记也不永久排除(§needs N1)。
5. **没有校验全部 20 594 个文件**: 只校验了 84 个(§7)。全量需要 9.371 GiB 下载,
   属于"素材真正落地"时的动作, 不该在开发机上跑。
6. **`needs`(SFZ opcode 白名单)字段原样搬运, 未做语义校验**: 未验证这些 opcode 是否都被
   `crates/yeban-sfz` 支持 —— 那是 SFZ 解析器工作线的判据, 不该由素材登记工作线声称。
7. 未改 `scripts/**`、`schemas/**`、`.github/**`、根 `Cargo.toml`/`Cargo.lock`、`docs/adr/**`、
   `docs/YEBAN_*.md`、`docs/DEVELOPMENT_LEDGER.md`、其它 `crates/**`、
   `docs/ledger/{gate-status,phase-status,feature-alignment,human-decisions}.md`。

## 10. needs (请人类负责人 / 集成者裁决)

| # | needs | 建议 |
| ---: | :--- | :--- |
| **N1** | **`Unlicense` 素材(jSteelDrum, 372 文件 / 38 541 639 字节)是否入库?** | `ADR-0001 D20`"接受 Unlicense"是针对**代码依赖**的裁决, 不等于素材裁决, 故本工作线**不擅自**登记。建议: 要么(a)把 `Unlicense` 加进 `allowed_licenses` 并登记, 要么(b)明确排除并把原因写进 `ATTRIBUTION.md`。**当前状态: 既不登记也不判死, 悬空**(已写进 `excluded[]`, `commercial_usable: true`) |
| **N2** | **给许可白名单加机械判据**(改 `scripts/gates/validate_schemas.py`, 集成者独占) | 建议加: 读根清单 `allowed_licenses`(或清单自带 `licence_whitelist`), 逐条校验 `items[].license ∈ 白名单`。今天**任何**非商用素材塞进 `items[]` 都不会变红(§6 实测) |
| **N3** | **把拉取/校验脚本提升到 `scripts/dev/`** | 本工作线**禁改 `scripts/**`**, 所以生成器与校验器只能以原文形式嵌在本文件附录(§附录)。建议集成者把它们落到 `scripts/dev/gen-samples-manifest.py` 与 `scripts/dev/verify-samples-bytes.py`, 并可选加一个**手动档**CI job(联网、`workflow_dispatch`)做全量真字节校验 |
| **N4** | **293 款缺口的来源**(`HD-31`) | 需要人类负责人选定素材; 在此之前 `MUST-GATE-014` 只能记账为**部分**(30/323), 不得写成通过 |
| **N5** | **素材本体的分发形态** | 当前是"仓库只存指针 + 摘要, 用户自取"。若产品要求"开箱即用", 需要另一条分发通道(下载器 / 安装时拉取 / Release 附件), 这是产品决策 |
| **N6** | **`CC-BY-3.0` 进白名单的确认** | 根清单原 `allowed_licenses` 漏了 `CC-BY-3.0`, 本次因 Salamander Grand Piano 补上(规范原文只说「CC-BY」)。请确认这是否符合原意 |

## 11. 修改文件清单

| 文件 | 动作 | 说明 |
| :--- | :--- | :--- |
| `assets/samples/manifest.json` | **新增** | 8 903 874 字节, `category: samples`, 20 594 条 `items[]` + 30 条 `instruments[]` + 3 条 `excluded[]` |
| `assets/samples/ATTRIBUTION.md` | **重写** | 14 645 字节。⚠ 原文件含**未经核实的具体条目**(宣称 323 款、7 条来源/许可、以及"CI 校验 RIFF chunk"等**并不存在**的机制), 已整体替换为由清单派生的真实内容 |
| `assets/manifest.json` | 改 | 给 `samples` 分类挂上 `manifest` 指针; 白名单补 `CC-BY-3.0`; 加 `instruments_registered`/`instruments_shortfall`/`bundled`; 重写 `note` |
| `crates/yeban-sfz/src/lib.rs` | 改 | 文档注释里"`assets/samples/` 目前只有 ATTRIBUTION.md"已过时, 更新为"登记式清单已就位、字节不入库、30/323" |
| `docs/ledger/samples-attribution-notes.md` | **新增** | 本文件 |

## 附录 A: 生成器原文 (`gen_samples_manifest.py`)

本工作线**禁改 `scripts/**`**, 因此生成器以原文形式留档。用法:
`python3 gen_samples_manifest.py <groove-manifest.json> <out-manifest.json>`。

```python
#!/usr/bin/env python3
"""把 groove 的 samples manifest.json 转成 yeban 的 assets/samples/manifest.json。

设计要点（详见 docs/ledger/samples-attribution-notes.md）:
  * 清单是**登记式(registry-only)**: 官方仓库不随包分发任何采样字节, 所有 item 带
    `optional: true`。schema 与门禁都明确支持这一形态("登记义务, 不是打包义务")。
  * 每个**文件**一条 item(与 assets/brand/manifest.json 的既有约定一致: 一文件一条),
    保留 sha256 + size_bytes + license。
  * 乐器级元数据(licence/repo/pin/sfz/needs/体积)放在顶层 `instruments`。
  * 非白名单许可(CC-BY-NC-SA / CC-Sampling-Plus / Unlicense-待裁决)只进 `excluded`,
    **绝不进 items**。
  * 确定性: 输入同一份 groove manifest ⇒ 输出**逐字节相同**。所有列表显式排序,
    字段顺序由构造顺序固定, 不做 sort_keys(那样会打乱可读的字段顺序)。

用法:
    python3 gen_samples_manifest.py <groove-manifest.json> <out-manifest.json>
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

GROOVE_MANIFEST = Path(sys.argv[1])
OUT = Path(sys.argv[2])

SPEC_TARGET_INSTRUMENTS = 323  # MUST-GATE-014 的规范目标(款)

# 许可白名单 —— 规范原文: "仅限 CC0/CC-BY/MIT"。
# CC-BY-3.0 与 CC-BY-4.0 都在 CC-BY 家族内, 故都列入(见 notes: 根清单原本只写了 4.0)。
WHITELIST = ["CC0-1.0", "CC-BY-3.0", "CC-BY-4.0", "MIT", "Apache-2.0"]

# 上游 literal licence -> (SPDX, commercial_usable, attribution_required)
LICENCE_MAP = {
    "CC0": ("CC0-1.0", True, False),
    "CC-BY": ("CC-BY-4.0", True, True),  # 逐乐器覆盖(见 LICENCE_OVERRIDE)
    "CC-BY-NC-SA": ("CC-BY-NC-SA", False, True),
    "CC-Sampling-Plus": ("CC-Sampling-Plus-1.0", True, True),
    "Unlicense": ("Unlicense", True, False),
}

# 上游 literal "CC-BY" 无法区分版本 —— 逐乐器查上游核实后的精确 SPDX 与署名对象。
# 证据: 各 repo 的 README/LICENSE(GitHub License API + raw README), 见 notes §来源与核实。
LICENCE_OVERRIDE = {
    "salamander-grand": ("CC-BY-3.0", "Alexander Holm"),
    "mtg-solo-sax": ("CC-BY-4.0", "MTG (Music Technology Group, Universitat Pompeu Fabra); SFZ 转换 kinwie"),
    "ixox-flute": ("CC-BY-4.0", "Xavier Hosxe"),
}

# 过滤原因(逐条, 会写进 ATTRIBUTION.md 与 notes)
EXCLUDE_REASON = {
    "jlearman-jrhodes3c": "CC-BY-NC-SA: 含**非商业**限制 ⇒ 违反 AGENTS.md §2 红线 2 与 MUST-GATE-014 的 CC0/CC-BY/MIT 白名单, 不得入库。",
    "sonatina-brass": "CC-Sampling-Plus: 不在 MUST-GATE-014 白名单(CC0/CC-BY/MIT)内 ⇒ 不得入库, 即使它允许商用。",
    "jlearman-steel-drum": "Unlicense: 不在白名单内, 且 `ADR-0001 D20` 接受 Unlicense 是针对**代码依赖**的裁决, 不等于素材裁决 ⇒ **待人类负责人裁决**(见 needs N1)。",
}


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _indent(value: object, pad: str) -> str:
    """把 json.dumps 的结果整体右移, 便于嵌进手写的外层结构。"""
    text = json.dumps(value, ensure_ascii=False, indent=2)
    return "\n".join(pad + line if line else line for line in text.split("\n"))


#: G06(policy_check.py) 硬上限: 任何 >10 MiB 的文件都会红, 且 LARGE_FILE_ALLOWLIST 在
#: scripts/(本工作线禁改)里且为空。所以**清单必须压到 10 MiB 以下** —— 这不是美学问题。
MAX_MANIFEST_BYTES = 10 * 1024 * 1024


def render(doc: dict) -> str:
    """手写序列化: 顶层字段与 instruments/excluded 保持缩进可读, `items` 每个一条**单行** JSON。

    为什么要单行: (1) 20 594 条 item 用 indent=2 会到 10.6 MB > G06 的 10 MiB 上限;
    (2) 一条 item 一行时, 某个文件的 sha256 变了就是**一行 diff**, 而不是 12 行。
    """
    keys = [
        "$schema",
        "category",
        "version",
        "policy",
        "upstream",
        "licence_whitelist",
        "licence_whitelist_source",
        "licence_whitelist_enforced_by_gate",
        "licence_whitelist_enforcement_note",
        "counts",
        "counts_note",
    ]
    parts = [f'  "{k}": {_indent(doc[k], "  ")}' for k in keys]
    parts.append(f'  "instruments": {_indent(doc["instruments"], "  ")}')
    parts.append(f'  "excluded": {_indent(doc["excluded"], "  ")}')
    items = doc["items"]
    body = ",\n".join(
        "    " + json.dumps(it, ensure_ascii=False, separators=(",", ":")) for it in items
    )
    parts.append(f'  "items": [\n{body}\n  ]')
    text = "{\n" + ",\n".join(parts) + "\n}\n"
    if len(text.encode()) > MAX_MANIFEST_BYTES:
        raise SystemExit(
            f"清单 {len(text.encode())} 字节 > G06 上限 {MAX_MANIFEST_BYTES} —— "
            "policy_check.py G06 会判红, 必须先把清单压下来"
        )
    return text


def main() -> int:
    raw = GROOVE_MANIFEST.read_bytes()
    upstream_sha = hashlib.sha256(raw).hexdigest()
    doc = json.loads(raw)
    entries = sorted(doc["entries"], key=lambda e: e["id"])

    instruments = []
    excluded = []
    items = []

    bytes_registered = 0
    bytes_excluded = 0
    files_registered = 0
    files_excluded = 0

    for entry in entries:
        eid = entry["id"]
        prefix = entry.get("prefix") or eid
        upstream_licence = entry["licence"]
        files = sorted(entry["files"], key=lambda f: f["path"])
        total = sum(f["bytes"] for f in files)

        is_excluded = eid in EXCLUDE_REASON
        if is_excluded:
            files_excluded += len(files)
            bytes_excluded += total
            excluded.append(
                {
                    "id": eid,
                    "name": entry["name"],
                    "licence_upstream": upstream_licence,
                    "commercial_usable": upstream_licence != "CC-BY-NC-SA",
                    "reason": EXCLUDE_REASON[eid],
                    "repo": entry.get("repo"),
                    "pin": entry.get("pin"),
                    "source_url": entry.get("sourceUrl"),
                    "sfz": entry.get("sfz"),
                    "file_count": len(files),
                    "total_bytes": total,
                    "in_items": False,
                }
            )
            continue

        spdx, commercial, attribution = LICENCE_MAP[upstream_licence]
        author = (entry.get("repo") or entry.get("sourceUrl") or "见 source_url").split("/")[-1]
        if eid in LICENCE_OVERRIDE:
            spdx, author = LICENCE_OVERRIDE[eid]
        assert spdx in WHITELIST, f"{eid}: {spdx} 不在白名单 {WHITELIST} 内"

        rel_root = f"assets/samples/{prefix}"
        # 乐器指纹(MUST-GATE-014 要"每款乐器的指纹"): 定义 = 对该乐器**按路径排序**的每个文件,
        # 依次拼接 "<相对路径>\n<sha256>\n" 后取 SHA-256。口径写进 ATTRIBUTION.md, 可独立复算。
        fingerprint = hashlib.sha256(
            "".join(f"{f['path']}\n{f['sha256']}\n" for f in files).encode()
        ).hexdigest()
        inst = {
            "id": eid,
            "prefix": prefix,
            "name": entry["name"],
            "fingerprint": fingerprint,
            "fingerprint_recipe": "sha256( concat( path + '\\n' + sha256 + '\\n' for each file, sorted by path ) )",
            "license": spdx,
            "licence_upstream": upstream_licence,
            "commercial_usable": commercial,
            "attribution_required": attribution,
            "author": author,
            "provenance_form": "archive" if entry.get("archive") else "git",
            "repo": entry.get("repo"),
            "pin": entry.get("pin"),
            "source_url": entry.get("sourceUrl"),
            "category": entry.get("category"),
            "sfz": entry.get("sfz"),
            "relative_root": rel_root,
            "file_count": len(files),
            "total_bytes": total,
            "bundled": False,
            "needs": entry.get("needs", []),
        }
        if entry.get("archive"):
            inst["archive"] = entry["archive"]
        instruments.append(inst)

        files_registered += len(files)
        bytes_registered += total

        for f in files:
            item_id = f"{prefix}/{f['path']}"
            items.append(
                {
                    "id": item_id,
                    "name": f["path"].rsplit("/", 1)[-1],
                    "instrument": eid,
                    "relative_path": f"assets/samples/{item_id}",
                    "sha256": f["sha256"],
                    "size_bytes": f["bytes"],
                    "license": spdx,
                    "commercial_usable": commercial,
                    "attribution_required": attribution,
                    "optional": True,
                }
            )

    items.sort(key=lambda i: i["id"])
    instruments.sort(key=lambda i: i["id"])
    excluded.sort(key=lambda i: i["id"])

    doc_out = {
        "$schema": "../../schemas/assets.manifest.schema.json",
        "category": "samples",
        "version": "1.0.0",
        "policy": (
            "登记式(registry-only): 官方仓库**不随包分发**任何采样字节。总素材 10 061 840 365 字节 "
            "(9.371 GiB)/21 505 个文件, 超过 GitHub 单次 push 2 GB 上限, 也不该让每次 clone 背 9.4 GiB。"
            "因此每条 item 带 optional=true: 门禁不因文件缺失报错, 但会打印未随仓库分发的条数。"
            "素材由使用者按 instruments[].repo + pin(或 archive.url)自行拉取, 并用本清单的 sha256 校验。"
        ),
        "upstream": {
            "source_repo": "apple2011:music/groove (本机兄弟工作树, 非 yeban 仓库的一部分)",
            "source_manifest": "groove/public/samples/manifest.json",
            "source_manifest_bytes": len(raw),
            "source_manifest_sha256": upstream_sha,
            "source_manifest_version": doc.get("version"),
            "captured_at": "2026-10-05",
            "transform": "scripts 未入库, 生成器原文见 docs/ledger/samples-attribution-notes.md §转换规则",
        },
        "licence_whitelist": WHITELIST,
        "licence_whitelist_source": "docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:390 (MUST-GATE-014: 仅限 CC0/CC-BY/MIT)",
        "licence_whitelist_enforced_by_gate": False,
        "licence_whitelist_enforcement_note": (
            "**已知缺口**: scripts/gates/validate_schemas.py 只校验结构 + SHA-256/大小 + 未登记文件, "
            "**不读** licence, 也不读根清单的 allowed_licenses(全仓库无任何代码读它们)。"
            "实测: 注入一条 license=CC-BY-NC-SA 的 item 后 --repo-assets 依旧绿。"
            "补上这条机械判据需要改 scripts/(集成者独占), 见 notes §needs N2。"
        ),
        "counts": {
            "instruments_upstream": len(entries),
            "instruments_registered": len(instruments),
            "instruments_excluded": len(excluded),
            "instruments_spec_target": SPEC_TARGET_INSTRUMENTS,
            "instruments_shortfall_vs_spec": SPEC_TARGET_INSTRUMENTS - len(instruments),
            "files_registered": files_registered,
            "files_excluded": files_excluded,
            "files_upstream": files_registered + files_excluded,
            "bytes_registered": bytes_registered,
            "bytes_excluded": bytes_excluded,
            "bytes_upstream": bytes_registered + bytes_excluded,
        },
        "counts_note": (
            f"规范 MUST-GATE-014 要求 {SPEC_TARGET_INSTRUMENTS} 款原声乐器; 本次**只**登记 "
            f"{len(instruments)} 款(27 CC0 + 3 CC-BY), **差 {SPEC_TARGET_INSTRUMENTS - len(instruments)} 款**。"
            f"这不是 323, 也不得写成 323。"
        ),
        "instruments": instruments,
        "excluded": excluded,
        "items": items,
    }

    OUT.parent.mkdir(parents=True, exist_ok=True)
    text = render(doc_out)
    OUT.write_text(text, encoding="utf-8")
    # 自证: 产出的文本必须是合法 JSON(手写序列化必须自己验, 不能假设)
    assert json.loads(text)["counts"]["files_registered"] == files_registered

    print(f"写出 {OUT}")
    print(f"  instruments: {len(instruments)} (excluded {len(excluded)})")
    print(f"  items(files): {len(items)}  (excluded files {files_excluded})")
    print(f"  bytes_registered: {bytes_registered}  bytes_excluded: {bytes_excluded}")
    print(f"  manifest bytes: {len(text.encode())}  sha256: {hashlib.sha256(text.encode()).hexdigest()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

```

## 附录 B: 校验脚本原文 (`verify_samples_bytes.py`)

用法: `python3 verify_samples_bytes.py <repo-root>`(会真的从上游 pin 下载字节并重算 sha256)。

```python
#!/usr/bin/env python3
"""真字节校验: 从**上游 pin** 下载真实文件, 重算 SHA-256/大小, 与本仓库清单对账。

为什么必须做这件事(而不是只跑门禁):
    门禁对 20 594 条 item 全部跳过(optional=true, 磁盘上没有字节), 它打印的是
    "0/20594 条资产的 SHA-256 与磁盘一致(另有 20594 项 optional 资产未随仓库分发)"。
    也就是说 **门禁此刻一个字节都没校验**。只登记 sha256 而从不校验真实字节, 等于把
    "登记"变成不可证伪的自我声明 —— 本脚本存在的唯一目的就是不那样干。

覆盖面(>=20 个文件, 含全局最大与最小):
    * 全局最小(0 字节) 与 全局最大(7.8 MB) 各 1 个;
    * **每个** git 溯源乐器的 .sfz 入口文件(全乐器覆盖);
    * 每种音频格式各取最小的一个(wav/flac/ogg/mp3) —— 证明校验的不只是文本;
    * 两个 archive 溯源乐器(FreePats 风琴)走 tarball: 先校验 archive.sha256,
      再解包校验成员文件的 sha256(这是另一种溯源形态, 必须同等对待)。

用法:
    python3 verify_samples_bytes.py <repo-root-with-assets-samples-manifest.json>
"""

from __future__ import annotations

import hashlib
import io
import json
import sys
import tarfile
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

REPO = Path(sys.argv[1]).resolve()
MANIFEST = REPO / "assets/samples/manifest.json"
RAW = "https://raw.githubusercontent.com/{repo}/{pin}/{path}"
TIMEOUT = 600
CACHE = Path("/tmp/yeban-samples/cache")
#: 大 archive(255 MB / 103 MB)只校验整包 SHA-256 —— 整包摘要一致即**传递地**确认了每个成员的
#: 字节(成员清单本来就来自同一个包)。小 archive 才额外逐成员校验。
MEMBER_VERIFY_MAX_BYTES = 32 * 1024 * 1024


def fetch(url: str) -> bytes:
    """带磁盘缓存: 一次校验要拉 ~370 MB, 重跑时不该再拉一遍。"""
    CACHE.mkdir(parents=True, exist_ok=True)
    cached = CACHE / hashlib.sha256(url.encode()).hexdigest()
    if cached.is_file():
        return cached.read_bytes()
    request = urllib.request.Request(url, headers={"User-Agent": "yeban-sample-verify/1"})
    with urllib.request.urlopen(request, timeout=TIMEOUT) as response:  # noqa: S310 - 固定 https
        blob = response.read()
    cached.write_bytes(blob)
    return blob


def raw_url(repo: str, pin: str, path: str) -> str:
    return RAW.format(repo=repo, pin=pin, path=urllib.parse.quote(path, safe="/"))


def main() -> int:
    doc = json.loads(MANIFEST.read_text(encoding="utf-8"))
    items = doc["items"]
    instruments = {i["id"]: i for i in doc["instruments"]}

    # ---- 选样 -----------------------------------------------------------------
    by_size = sorted(items, key=lambda i: (i["size_bytes"], i["id"]))
    picked: dict[str, dict] = {}
    picked["GLOBAL-MIN"] = by_size[0]
    picked["GLOBAL-MAX"] = by_size[-1]

    # 每个 git 乐器一个 .sfz 入口
    for inst in doc["instruments"]:
        if inst["provenance_form"] != "git":
            continue
        sfz = inst["sfz"]
        match = next((i for i in items if i["instrument"] == inst["id"] and i["id"].endswith(sfz)), None)
        if match:
            picked[f"SFZ:{inst['id']}"] = match

    # 每种音频格式最小的一个
    ext_min: dict[str, dict] = {}
    for item in by_size:
        ext = item["name"].rsplit(".", 1)[-1].lower()
        if ext in {"wav", "flac", "ogg", "mp3"} and ext not in ext_min:
            ext_min[ext] = item
    for ext, item in ext_min.items():
        picked[f"FMT-MIN:{ext}"] = item

    # ---- 逐个下载对账 ---------------------------------------------------------
    print(f"清单: {MANIFEST}")
    print(f"清单 sha256: {hashlib.sha256(MANIFEST.read_bytes()).hexdigest()}")
    print(f"选样: {len(picked)} 个文件\n")
    header = f"{'标签':<34} {'期望 sha256':<14} {'实际 sha256':<14} {'期望字节':>10} {'实际':>10}  判定"
    print(header)
    print("-" * len(header))
    failures: list[str] = []
    checked = 0
    for label, item in picked.items():
        inst = instruments[item["instrument"]]
        # 注意: item id 用的是 **prefix**, 而 instruments[].id 是 entry id —— 两者可能不同
        # (实测 virtuosity-drums-basic 的 prefix 是 virtuosity-drums)。用 prefix 切才对。
        url = raw_url(inst["repo"], inst["pin"], item["id"][len(inst["prefix"]) + 1 :])
        try:
            blob = fetch(url)
        except Exception as error:  # noqa: BLE001
            print(f"{label:<34} {item['sha256'][:12]:<14} {'--':<14} {item['size_bytes']:>10} {'--':>10}  DOWNLOAD-ERR {error}")
            failures.append(f"{label}: 下载失败 {error}")
            continue
        digest = hashlib.sha256(blob).hexdigest()
        good = digest == item["sha256"] and len(blob) == item["size_bytes"]
        print(
            f"{label:<34} {item['sha256'][:12]:<14} {digest[:12]:<14} "
            f"{item['size_bytes']:>10} {len(blob):>10}  {'OK' if good else 'MISMATCH'}"
        )
        if good:
            checked += 1
        else:
            failures.append(f"{label}: sha/大小不符")

    # ---- archive 溯源乐器 -----------------------------------------------------
    print("\n-- archive 溯源乐器(FreePats): 先校验 tarball, 再校验成员 --")
    for inst in doc["instruments"]:
        archive = inst.get("archive")
        if not archive:
            continue
        print(f"{inst['id']}: {archive['asset']} 期望 sha256 {archive['sha256'][:12]}…")
        try:
            blob = fetch(archive["url"])
        except Exception as error:  # noqa: BLE001
            print(f"  DOWNLOAD-ERR {error}")
            failures.append(f"{inst['id']}: archive 下载失败")
            continue
        digest = hashlib.sha256(blob).hexdigest()
        ok = digest == archive["sha256"] and len(blob) == archive["bytes"]
        print(f"  tarball 实际 {digest[:12]}… {len(blob)} 字节  {'OK' if ok else 'MISMATCH'}")
        if not ok:
            failures.append(f"{inst['id']}: archive sha/大小不符")
            continue
        members = {
            i["id"][len(inst["prefix"]) + 1 :]: i for i in items if i["instrument"] == inst["id"]
        }
        if len(blob) > MEMBER_VERIFY_MAX_BYTES:
            print(
                f"  整包摘要一致 ⇒ 传递地确认了 {len(members)} 个成员的全部字节; "
                f"({len(blob)} 字节 > {MEMBER_VERIFY_MAX_BYTES} 阈值, 不再逐成员解包)"
            )
            continue
        if archive["asset"].endswith(".zip"):
            container = zipfile.ZipFile(io.BytesIO(blob))
            names = {n: n for n in container.namelist()}
            reader = lambda n: container.read(n)  # noqa: E731
        else:
            tar = tarfile.open(fileobj=io.BytesIO(blob), mode="r:xz")
            names = {m.name.lstrip("./"): m for m in tar.getmembers() if m.isfile()}
            reader = lambda n: tar.extractfile(names[n]).read()  # noqa: E731
        member_ok = 0
        member_all = 0
        for path, item in members.items():
            if path not in names:
                continue
            member_all += 1
            data = reader(path)
            member_digest = hashlib.sha256(data).hexdigest()
            if member_digest == item["sha256"] and len(data) == item["size_bytes"]:
                member_ok += 1
            else:
                failures.append(f"{inst['id']}/{path}: 成员 sha/大小不符")
        print(f"  成员文件校验: {member_ok}/{member_all} 与清单一致 (清单共 {len(members)} 条)")
        checked += member_ok

    print(f"\n结果: {checked} 个文件的真实字节与清单一致; 失败 {len(failures)} 条")
    for f in failures:
        print(f"  - {f}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())

```

## 附录 C: 注入测试脚本原文 (`injections.sh`)

用法: `bash injections.sh`。

```bash
#!/usr/bin/env bash
# 判据注入测试: 每条都做到 "注入 -> 变红 -> 还原 -> 变绿", 并留下原始输出与退出码。
# 注意: 门禁命令**不做任何管道/head/tail**(L6), 退出码逐个捕获。
set -uo pipefail

REPO="/Users/crow/work/music/yeban/.worktrees/samples-attribution"
MAN="$REPO/assets/samples/manifest.json"
BACKUP="/tmp/yeban-samples/manifest.final.json"
cd "$REPO"

step() { printf '\n\033[1m### %s\033[0m\n' "$*"; }

gate() {
  step "运行: python3 scripts/gates/validate_schemas.py --repo-assets"
  python3 scripts/gates/validate_schemas.py --repo-assets
  echo "EXIT=$?"
}

guards() {
  step "运行: python3 scripts/guards/policy_check.py (G06)"
  python3 scripts/guards/policy_check.py
  echo "EXIT=$?"
}

cp "$MAN" "$BACKUP"
echo "备份清单: $(shasum -a 256 "$BACKUP" | cut -d' ' -f1)"

# ---------------------------------------------------------------- ③ 未登记文件
step "判据 ③ 注入: 放一个未登记文件 assets/samples/INJECT_kick.wav"
printf 'RIFF____WAVEfmt ' > assets/samples/INJECT_kick.wav
ls -l assets/samples/INJECT_kick.wav
gate

step "判据 ③ 还原: 删除 INJECT_kick.wav"
rm assets/samples/INJECT_kick.wav
gate

# ---------------------------------------------------------------- ④ 非白名单许可
step "判据 ④ 注入: 往 items 里加一条 license=CC-BY-NC-SA 的**结构合法**条目"
python3 - <<'PY'
import json, pathlib
p = pathlib.Path("/Users/crow/work/music/yeban/.worktrees/samples-attribution/assets/samples/manifest.json")
text = p.read_text(encoding="utf-8")
item = {
    "id": "jlearman-jrhodes3c/_jRhodes-both-looped.sfz",
    "name": "_jRhodes-both-looped.sfz",
    "instrument": "jlearman-jrhodes3c",
    "relative_path": "assets/samples/jlearman-jrhodes3c/_jRhodes-both-looped.sfz",
    "sha256": "0" * 64,
    "size_bytes": 1,
    "license": "CC-BY-NC-SA",
    "commercial_usable": False,
    "attribution_required": True,
    "optional": True,
}
idx = text.rindex("\n  ]\n}")
p.write_text(text[:idx] + ",\n    " + json.dumps(item, ensure_ascii=False, separators=(",", ":")) + text[idx:], encoding="utf-8")
print("已注入; items 数 =", len(json.loads(p.read_text(encoding='utf-8'))["items"]))
PY
gate

step "判据 ④ 关键读数: 门禁**不读** license —— 注入非白名单许可后是否仍然绿?"
python3 - <<'PY'
import json
doc = json.load(open("/Users/crow/work/music/yeban/.worktrees/samples-attribution/assets/samples/manifest.json"))
bad = [i["id"] for i in doc["items"] if i.get("license") not in doc["licence_whitelist"]]
print("清单里非白名单许可的条目:", bad)
print("门禁源码里有没有读 licence 的地方:")
PY
grep -c "license" scripts/gates/validate_schemas.py
echo "(validate_schemas.py 中 license 出现次数, 上面这个数字 ^)"

step "判据 ④ 用自己的白名单检查(真判据): 非白名单许可必须红"
python3 - <<'PY'
import json, sys
d = json.load(open("/Users/crow/work/music/yeban/.worktrees/samples-attribution/assets/samples/manifest.json"))
wl = set(d["licence_whitelist"])
bad = [(i["id"], i["license"]) for i in d["items"] if i["license"] not in wl]
for i, lic in bad:
    print(f"  RED {i}: license={lic} 不在白名单 {sorted(wl)}")
print(f"非白名单条目 {len(bad)} 条")
sys.exit(1 if bad else 0)
PY
echo "EXIT=$?"

step "判据 ④ 还原清单"
cp "$BACKUP" "$MAN"
shasum -a 256 "$MAN"
gate

# ---------------------------------------------------------------- G06 有牙
step "附加: G06(>10MB 文件)确实有牙 —— 放入 10.6 MB 的**缩进版**清单"
python3 - <<'PY'
import json
doc = json.load(open("/tmp/yeban-samples/manifest.final.json"))
text = json.dumps(doc, ensure_ascii=False, indent=2) + "\n"
open("/Users/crow/work/music/yeban/.worktrees/samples-attribution/assets/samples/manifest.json", "w").write(text)
print("缩进版字节数 =", len(text.encode()), " > 上限", 10 * 1024 * 1024)
PY
guards

step "附加: 还原紧致版清单"
cp "$BACKUP" "$MAN"
shasum -a 256 "$MAN"
guards
gate

step "全部注入测试完成; 清单 sha256 应等于 218a… 之后的值(见上)"

```
