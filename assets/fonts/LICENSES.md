# 界面字体许可与登记说明 (Font Licenses & Registry)

本目录 (`assets/fonts/`) 是**界面字体**的落点: 许可与署名文档在这里
(本文件 + [`OFL-1.1.md`](OFL-1.1.md)), 机器可读的登记在 [`manifest.json`](manifest.json),
根指针在 [`../../assets/manifest.json`](../manifest.json) 的 `category: "fonts"` 一条。

## 0. 本文件的一处**更正** (2026-10-08)

本文件在 2026-10-08 之前写着「Inter 4.0+」「JetBrains Mono 2.304+」两支族, 并在末尾写
「所有字体文件的完整原始许可证文本保留于字体所在目录中」。当时的**事实**是:
这个目录里**一个字体文件都没有** (只有本文件), 而 `crates/yeban-app/ui/tokens.slint` 的
`font-ui` 用的是 **macOS 系统族 `PingFang SC`**, 不是 Inter。也就是说旧文本是一条
**没有载体的登记**: 登记了字节, 字节不在; 登记了许可文本, 文本不在。

负责人于 2026-10-08 下令「打包字体」后, 本文件按**实际在树里的字节**重写:

- 「Inter」这条**删除** —— 仓库里从来没有 Inter 的字节, `.slint` 侧也从来没有引用过它
  (机械读数: 对 `crates/yeban-app/ui/**` 搜 `Inter` = 0 命中); 它是一句从未兑现的自述,
  留着就是假账。
- 「JetBrains Mono」这条**兑现**了: 字节现在真的在树里 (§1)。
- 「Noto Serif SC」**新增**: 它是负责人点名的第三支, 旧文件里从未登记过。
- 末句「许可文本保留于字体所在目录中」现在**成立**: 见 [`OFL-1.1.md`](OFL-1.1.md)。

## 1. 随仓库分发的字体 (bundled — 字节在树里)

| # | 族内名 (name ID 1) | 字重 | 文件 | 字节 | sha256 |
| -: | :--- | ---: | :--- | ---: | :--- |
| 1 | `JetBrains Mono` | Regular / 400 | [`JetBrainsMono-Regular.ttf`](JetBrainsMono-Regular.ttf) | 273900 | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` |
| 2 | `Noto Serif SC` | Regular / 400 | [`NotoSerifSC-Regular-subset.otf`](NotoSerifSC-Regular-subset.otf) | 8888964 | `4293341c25ff2cf9220025114289088184a367e784f9655db4773e07770f5f55` |

上游来源 (逐条可复算):

| # | 上游 URL | 上游 sha256 | 上游字节 | 本仓文件的变换 |
| -: | :--- | :--- | ---: | :--- |
| 1 | `https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip` (内含 `fonts/ttf/JetBrainsMono-Regular.ttf`) | zip: `6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf` | zip: 5622857 | **无**: 解出来的那个 `.ttf` 一位未改 |
| 2 | `https://github.com/notofonts/noto-cjk/raw/main/Serif/SubsetOTF/SC/NotoSerifSC-Regular.otf` | `e8f396decc1f0963a016a989c3d8852e863d1350996f573860a80767c83a1cd3` | 11625800 | **pyftsubset 删字形** (§3) |

两族都是 **OFL-1.1**, 因此都 `commercial_usable: true`。OFL-1.1 允许打包、嵌入、修改与
**商用再分发**; 唯一的实质义务是**随附版权声明与许可文本** —— 那一段文字在
[`OFL-1.1.md`](OFL-1.1.md), **两份上游原文逐字节带全** (核验过: 与上游文件逐字节相同)。
两支上游的 OFL 版权行里**都没有** "with Reserved Font Name …" 子句 ⇒ RFN 义务不触发,
修改过的副本可以继续用原族名。

`JetBrains Mono` 的版本 = **2.304** (上游 ZIP 的 tag 即 `v2.304`)。

## 2. 为什么只带一个真字面 (Regular / 400)

机械读数 (对 `crates/yeban-app/ui/**` 搜 `font-weight`): 全仓只有 **5** 处写 `font-weight`,
全部在 `tokens.slint` 里 —— `UiText` 400 / `UiMediumText` 500 / `UiTitleText` 600,
而这三支的 `font-family` 是 **`Tokens.font-ui` (= `PingFang SC`, 系统族)**; 用到打包两族的
三个位置**一处都没有**写 `font-weight`:

- `UiMonoText` (`tokens.slint`, 全界面 36 处使用) —— `font-family: Tokens.font-mono`, 无 `font-weight`;
- `transport.slint:217` 的 BPM `TextInput` —— `font-family: Tokens.font-mono`, 无 `font-weight`;
- `UiSerifText` (`tokens.slint`, 全界面**只有** `arrangement_view.slint:108` 一处 —— 章节名) ——
  `font-family: Tokens.font-serif`, 无 `font-weight`。

⇒ 这两族在本 UI 上**只被请求 400**。带 `Bold` / `Medium` 的字面是**没有使用点**的字节:
`JetBrains Mono` 每个字面约 274 KB, 而 `Noto Serif SC` 每个多带一个真字面大约要再花
**3–4 MB** (CJK 字面共享的字形骨架在 CFF 里虽然能省, 但每字面仍各有 charstring)。
因此本次**只带 Regular**, 并把这件事登记在 `manifest.json` 的 `note` 里 ——
若日后 `UiMonoText` / `UiSerifText` 要加 `font-weight`, **必须同时**补打包对应字面,
否则 Slint 会退回合成加粗 (模糊) 或落到系统兜底族。

## 3. `Noto Serif SC` 的子集: 口径与复现 (这是唯一一个**改过**的字节)

上游 `Serif/SubsetOTF/SC/NotoSerifSC-Regular.otf` 是 **11625800 字节 = 11.09 MiB**,
超过本仓自己的单文件上限 **10 MiB** (`scripts/guards/policy_check.py:71` 的
`MAX_FILE_BYTES = 10 * 1024 * 1024`, 而 `:73` 的 `LARGE_FILE_ALLOWLIST = ()` 是空的)。
⇒ 要么改上限 (动守卫 = 削弱门禁, 本切片不允许), 要么**只带用得上的字形**。选后者。

**口径 (一句话)**: 保留上游 SC 子集 OTF 的**全部码位, 只去掉两个 CJK 扩展区** ——
CJK Ext A (`U+3400–U+4DBF`) 与 Ext B+ (`U+20000` 以上)。结果是
**24119 / 30928** 个码位 (上游去掉 6809 个), 文件从 11.09 MiB 降到 **8.48 MiB**,
留 1.5 MiB 余量。`name` 表 / CFF 轮廓 / layout 特性 / hinting **一字未动**,
族内名仍是 `Noto Serif SC` / `Regular`。

复现命令 (本仓外执行; `fontTools` 4.58.5, `pyftsubset` 是它的 CLI):

```bash
# ① 取上游那一份 (sha256 e8f396decc1f0963a016a989c3d8852e863d1350996f573860a80767c83a1cd3)
curl -sSLo NotoSerifSC-Regular.otf \
  https://github.com/notofonts/noto-cjk/raw/main/Serif/SubsetOTF/SC/NotoSerifSC-Regular.otf
# ② 码位表 = 上游 cmap 去掉 Ext A 与 Ext B+ (确定性, 无人工清单)
python3 - <<'PY'
from fontTools.ttLib import TTFont
cmap = sorted(TTFont("NotoSerifSC-Regular.otf", lazy=True).getBestCmap())
keep = [c for c in cmap if not (0x3400 <= c <= 0x4DBF) and not (0x20000 <= c <= 0x3FFFF)]
open("u-no-exta.txt", "w").write(",".join(f"U+{c:04X}" for c in keep))
PY
# ③ 子集 (默认参数: 保留 hinting / name 表 / layout 特性)
pyftsubset NotoSerifSC-Regular.otf \
  --output-file=NotoSerifSC-Regular-subset.otf --unicodes-file=u-no-exta.txt
sha256sum NotoSerifSC-Regular-subset.otf   # 应为 4293341c25ff2cf9220025114289088184a367e784f9655db4773e07770f5f55
```

**可复现性 (实测)**: 同一条命令跑两次, 输出**逐字节相同** (两次都是
`4293341c25ff2cf9220025114289088184a367e784f9655db4773e07770f5f55`) —— `pyftsubset`
默认**不**重算 `head.modified`, 所以这不是一次性的运气。

**覆盖 (实测, 逐码位查表)**: 子集 cmap 有 **24119** 个码位; **GB2312 的 7445 个字符
一个不缺** (`missing = 0`) ⇒ 任何能编成 GB2312 的简体中文正文都可正常排版。
**不去**的部分只有: CJK Ext A (6592 个码位在**上游 SC 子集里本来就存在**, 本次为体积删掉)、
Ext B+ (上游只有 217 个), 以及上游本来就没有的 (例如 Hangul `U+AC00–D7AF` = 0 个)。
命中不到的字形由 Slint 的兜底 (`[SansSerif, SystemUi]`, 见
`i-slint-common-1.18.1/sharedfontique.rs:188-192`) 出, 不是乱码方块 —— 但**不是衬线**。

**为什么不用 Google Fonts 的变量字体**: `ofl/notoserifsc/NotoSerifSC[wght].ttf`
= 25125512 字节; 先 `instancer` 出 wght=400 再同样子集, 得到的是 **10462348 字节**
= 9.98 MiB —— 离 10 MiB 上限只剩 **23 KB**, 任何一次上游改版都会让它越线。
CFF/OTF 这条路留 1.5 MiB 余量, 而 Slint 的字体栈 (fontique → skrifa) 支持 CFF。

## 4. Slint 怎么用上它们 (注册路径)

注册走的是 Slint 的**编译期**路径, **不是**运行时 API:

```slint
// crates/yeban-app/ui/tokens.slint 文件末尾 §5c (刻意放在末尾, 见那里的理由)
import "../../../assets/fonts/JetBrainsMono-Regular.ttf";
import "../../../assets/fonts/NotoSerifSC-Regular-subset.otf";
```

- 语法出处: `i-slint-compiler-1.18.1/parser/document.rs:296-299` 的 `/// import "something.ttf";`,
  以及 `pathutils.rs:14-16` 的 `is_font_file` (`.ttf` / `.ttc` / `.otf` 三种后缀);
- 收集: `object_tree.rs:237-260` 把字体 import 记进 `Document.custom_fonts`;
- 生成: `passes/collect_custom_fonts.rs` 为每个导出根生成
  `RegisterCustomFontByMemory` (嵌入) 或 `RegisterCustomFontByPath` (按路径) ——
  `passes.rs:364-371` 按 `embed_resources` 选;
- 本仓走**嵌入**: `slint-build` 的默认 `EmbedResourcesKind::EmbedFiles` 映射成
  `EmbedAllResources` (`slint-build-1.18.1/lib.rs:167-179`,
  `i-slint-compiler-1.18.1/lib.rs:242-244` 的 Rust 输出默认值) ⇒ **字节进二进制**,
  运行时**不**依赖 `assets/fonts/` 还在不在磁盘上。
- **不需要**新的 Rust 依赖: `yeban-app` 既有 `slint` / `slint-build` 依赖就够。
  运行时那条路 (`slint::fontique_011::shared_collection()`, `slint-1.18.1/lib.rs:734-780`)
  要开 `unstable-fontique-011` cargo feature, 本次**没有**用它。

⇒ `.slint` 里的 `Tokens.font-mono` / `Tokens.font-serif` 现在指向**随仓库分发的**族,
与装机字体集合无关 (这正是打包要买的性质: Linux/CI 上也有同一族, 于是
`crates/yeban-app/tests/golden/linux/**` 那 5 张基准的"族"那一半**不再**是空转)。

## 5. 本目录的机器可读登记

[`manifest.json`](manifest.json) 由 `scripts/gates/validate_schemas.py --repo-assets` 校验:
结构对 `schemas/assets.manifest.schema.json`, 每条 `items[]` 的 sha256 / size_bytes
**对磁盘重算**, 许可必须落在 `licence_whitelist` 内 (`["OFL-1.1"]`, 是根清单
`assets/manifest.json` 里 `fonts.allowed_licenses` 的子集), `commercial_usable` 不得为 false。
该脚本同时做**未登记资产扫描**: `assets/fonts/` 下任何非 `.md` / 非 `.json` 的文件
都必须在清单里 —— 本目录的两个字体文件都在, 两个文档文件按规则豁免。

> 注意: `.md` 豁免意味着**许可文档本身不在 sha256 对账范围内**。这是扫描器刻意的口径
> (文档不是"资产"), 本文件据此把两份 OFL 原文的内容**逐字节核验**写进了 §1 ——
> 核验是手工做的, 不是门禁做的。

## 6. 本次**没有**打包的东西 (如实登记)

1. **`font-ui` (`PingFang SC`) 的字面**: 不打包。它是 macOS 系统族, 本次任务的对象是
   负责人点名的**两支缺失族**; 打包一支 CJK 无衬线 (思源黑体/Noto Sans SC 同族)
   要再花 8–10 MB, 而且会**替换掉**负责人已经看过并接受的界面外观 —— 收益与代价
   不匹配。⇒ `font-ui` 在 Linux/CI 上仍然落到系统兜底族 (`system-ui` 一族),
   **这一半的不一致仍然存在**, 与本次改动之前一样。
2. **两支族的非 Regular 字重** (`Bold` / `Medium` / `SemiBold` / Italic / 变量轴):
   理由见 §2 (没有使用点)。
3. **`Inter`**: 见 §0 (仓库里从来没有它, `.slint` 里也从来没有引用过它)。
4. **`Noto Serif SC` 的 CJK Ext A / Ext B+**: 见 §3 (10 MiB 上限)。
5. **`Noto Serif SC` 的繁体 (TC) / 日文 (JP) / 韩文 (KR) 子集**: 上游 `SubsetOTF/` 下
   每个语言子集各是一份独立的 10 MB 级文件。本仓界面的衬线只用于**章节名**,
   简体 SC 子集已经覆盖其所需 (含全部 GB2312); 带三份区域子集要多花 ~25 MB,
   与本 UI 无关。**命中不到的字**由系统兜底族出。
