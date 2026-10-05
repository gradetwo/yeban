# 夜半 (Yeban) 原生乐器采样资产归属与合规清单 (Sample Library Attributions)

> 本文件用于严格追踪与声明夜半 (Yeban) 内置附带的 323 款原声乐器与出厂音色采样文件的来源、原始作者及授权协议。所有资产均经过知识产权审计，确保 100% 允许在开源与商业音乐制作场景中免版税使用。

---

## 1. 许可合规准入原则 (License Admission Criteria)

纳入夜半 (Yeban) 资产清单的采样库必须满足以下授权之一：
- **CC0 1.0 Universal (Public Domain)**：公有领域献出，完全无限制商用与再分发；
- **MIT / Apache-2.0 / BSD**：宽松开源许可，保留原始版权声明即可自由分发；
- **CC-BY 4.0 / CC-BY 3.0**：知识共享署名许可，必须在此文件中完整注明原作者与作品来源。

**严禁准入**：严禁纳入 CC-BY-NC（非商业）、CC-BY-ND（禁止演绎）以及从未经授权的商业采样库（如 Native Instruments Kontakt 商业库、Spitfire Audio 商业库等）中私自切片的任何音频文件。

---

## 2. 核心采样资产分类清单 (Sample Library Inventory)

| 资产标识 (Asset ID) | 乐器分类 | 预设名称 | 来源/项目名称 | 原始作者/贡献者 | 授权协议 (License) | 原始仓库/发布链接 |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `smpl-piano-upright-01` | 键盘类 (Keyboard) | Upright Piano Standard | Pianobook Open Library | Community Contributors | CC0 1.0 | https://www.pianobook.co.uk |
| `smpl-piano-grand-01` | 键盘类 (Keyboard) | Concert Grand 1928 | Versilian Studios VSCO 2 CE | Sam Gossner | CC0 1.0 | https://vis.versilstudios.com |
| `smpl-bass-p-01` | 贝斯类 (Bass) | P-Bass Finger & Slap | Virtual Playing Orchestra | Paul Battersby | CC-BY 3.0 / CC0 | http://virtualplaying.com |
| `smpl-drums-tr808` | 鼓机类 (Drums) | Vintage 808 Analog Circuit | FreePats Community | FreePats Sample Project | CC0 1.0 | https://freepats.zenvoid.org |
| `smpl-drums-tr909` | 鼓机类 (Drums) | Classic 909 Groove Kit | FreePats Community | FreePats Sample Project | CC0 1.0 | https://freepats.zenvoid.org |
| `smpl-strings-ens-01` | 弦乐类 (Strings) | Section Strings Sustain | No Buzz String Library | Philharmonia Orchestra | CC-BY 4.0 | https://philharmonia.co.uk |
| `smpl-synth-gs1-01` | 合成器类 (Synth) | FM Crystal EP | Dexed Community Cartridges | Dexed Open Collection | GPLv3 / CC0 | https://asb2m10.github.io/dexed |

*(注：全量 323 款乐器的细颗粒度 SHA-256 校验和与映射关系定义于 `assets/samples/manifest.json`)*

---

## 3. 自动化 CI 审计机制 (Automated CI Verification)

CI 流水线包含自动化检查步骤，验证：
1. `manifest.json` 中声明的每个采样文件均在此文件中拥有对应词条；
2. 采样音频元数据（RIFF Chunk）中无任何专有版权冲突信息；
3. 音频格式为合规的未压缩 PCM WAV 或 FLAC 格式。

---

## 怎么往仓库里加采样（机械约束，不是建议）

`assets/samples/` 目前**没有素材**（`MUST-GATE-014` 因此记账为 PENDING）：
门禁的**机制**已就绪，但入库哪些素材是**许可与付费决策**，由人类负责人选定（`HD-31`）。

加素材的步骤如下（`scripts/gates/validate_schemas.py --repo-assets` 会**强制**它们）：

1. **先建清单** `assets/samples/MANIFEST.json`（结构照 `assets/models/MANIFEST.json`）：
   `category: samples` + `version` + `policy` + `items`。
   注意 **items 不得为空** —— 校验器**刻意拒绝空清单**（空清单会让资产登记这条红线变成空转），
   所以清单必须与**首批条目一起**创建，不能先建一个空壳。
2. **每条素材登记**：`id` / `name` / `relative_path` / `sha256` / `license` / `commercial_usable`；
   需要署名的再加 `attribution_required` + `author` + `source_url`。
   允许的许可见根清单 `assets/manifest.json` 的 `allowed_licenses`（`CC0-1.0` / `CC-BY-4.0` / `MIT` / `Apache-2.0`）。
3. **在根清单里挂上指针**：把 `assets/manifest.json` 的 `sub_manifests` 里 `category: samples`
   那一项加上 `manifest: assets/samples/MANIFEST.json`。
4. **跑门禁**：`python3 scripts/gates/validate_schemas.py --repo-assets`。它会做三件事：
   ① 校验清单结构；② **逐项重算 SHA-256 与 size_bytes 并与磁盘对账**；
   ③ **扫描该目录下每一个非文档文件，凡未登记在 items 里的一律报错**
   （实测：丢一个 INJECT_kick.wav 进去会立刻红，删掉即绿）。

> 第 ③ 条是**红线 9 的机械形式**：只对账已登记的是不够的 —— 不这样扫一遍，
> 往目录里直接丢一个 wav 就能绕过每个二进制资产都要有许可与 SHA-256 记录这条要求。

