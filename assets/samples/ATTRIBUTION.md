# 夜半 (Yeban) 原声乐器采样资产归属与合规清单 (Sample Library Attributions)

> **本文件由 `assets/samples/manifest.json` 派生**(生成器见
> `docs/ledger/samples-attribution-notes.md` §转换规则)。手改本文件而不改清单会造成两者漂移,
> 而 `MUST-GATE-014` 要求的正是「清单与署名逐条吻合」。

## 0. 先说清楚规模: 登记了 30 款, 不是 323 款

- **规范目标**(`MUST-GATE-014`): **323 款**原声乐器指纹与本文件逐条 100% 吻合。
- **本次实际登记**: **30 款**(27 款 CC0 + 3 款 CC-BY), 共 20594 个文件 / 9844170377 字节。
- **差额**: 323 − 30 = **293 款未登记**。
- **另有 3 款被过滤掉**(非白名单许可, 共 911 个文件 / 217669988 字节), 见 §3。

> ⚠ **不得把 30 写成 323**。本文件与清单里没有任何「已完成 323」的说法; 这个差额是一行看得见的账。

## 1. 许可准入原则 (License Admission Criteria)

`MUST-GATE-014` 原文(`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:390`): 「仅限 CC0/CC-BY/MIT」。
本清单实际使用的白名单:

- `CC0-1.0`
- `CC-BY-3.0`
- `CC-BY-4.0`
- `MIT`
- `Apache-2.0`

**严禁准入**: CC-BY-NC / CC-BY-NC-SA 等带非商业限制的许可(违反 AGENTS.md §2 红线 2)、
CC-BY-ND、以及任何从商业采样库(如 Native Instruments Kontakt、Spitfire Audio)私自切片的音频。

> 说明: 根清单 `assets/manifest.json` 原先的 `allowed_licenses` 只列了 `CC-BY-4.0`, 漏了 `CC-BY-3.0`。
> 本次登记发现 Salamander Grand Piano 的上游许可是 **CC-BY 3.0 Unported**(见其 README), 规范原文写的是「CC-BY」而未限定版本, 故白名单补上 `CC-BY-3.0`。

## 2. 入库形态: 登记式 (registry-only), **零字节入库**

上游共 21505 个文件, 合计 **10061840365 字节(9.371 GiB)**。
这个体积**不复制进仓库**:

- GitHub 单次 push 上限 2 GB, 9.371 GiB 推不上去; 每次 clone 多背 9.4 GiB 也不可接受;
- 上游全部可公开拉取(见 §4 的 repo + pin), 仓库里存**指针与摘要**才是正确形态。

因此清单每条 `items[]` 都带 `optional: true`: 门禁**不因文件缺失报错**, 但会明确打印未随仓库分发的条数。
这是 schema 明确定义的语义 —— `optional` 是"登记义务, 不是打包义务"。

### 2.1 代价与风险(如实登记)

1. **门禁此刻一个字节都没校验**。它的原文输出是:
   `[ok] assets/samples/manifest.json: 0/20594 条资产的 SHA-256 与磁盘一致(另有 20594 项 optional 资产未随仓库分发)`。
   "绿"只意味着**清单结构与登记口径成立**, 不意味着素材字节可用。
2. **CI 不联网时无法校验任何采样字节**。本工作线已用真实下载做过抽样验证(见 §5),
   但那是一次性的本机动作, **不是** CI 的常规保护。
3. 素材落地后(无论用户手动还是 CI 拉取), 把对应条目的 `optional` 去掉,
   这条门禁就立刻从"登记检查"升级成"20 594 个文件的真字节对账"—— 无需改一行门禁代码。

## 3. 逐条署名 (Per-Instrument Attribution)

乐器指纹口径(可独立复算, 清单里每条 `instruments[]` 也带同一字段):

```text
sha256( concat( path + '\n' + sha256 + '\n' for each file, sorted by path ) )
其中 file 按 path 排序; path 为该乐器内的相对路径。
```

| # | 乐器 id (prefix) | 名称 | 许可 | 上游 repo @ pin | 文件数 | 字节 | 乐器指纹 | 入口 .sfz sha256 |
| ---: | :--- | :--- | :--- | :--- | ---: | ---: | :--- | :--- |
| 1 | `aliexpress-erhu` (`aliexpress-erhu`) | AliExpress erhu | `CC0-1.0` | `sfzinstruments/aliexpress-erhu@6615047b2fd06126877483e97b8bb4af9d00b080` | 191 | 92826029 | `627fb97f8cb23b27…` | `b9e18076031d` |
| 2 | `body-percussion` (`body-percussion`) | Body Percussion | `CC0-1.0` | `sfzinstruments/body_percussion@4ac9d8966679c648b62fa10a188179e186b97f24` | 233 | 61733852 | `8f96e8145860f00b…` | `60cbbe738b5d` |
| 3 | `cithara-barbarica` (`cithara-barbarica`) | Cithara barbarica | `CC0-1.0` | `sfzinstruments/cithara-barbarica@a47c10dc4a26538a8a56d31d3436488138292ca2` | 275 | 239034087 | `b83dcd3d72a9137b…` | `dac346a0a1c9` |
| 4 | `discord-gm-sitar` (`discord-gm-sitar`) | Discord SFZ GM Bank — Sitar | `CC0-1.0` | `sfzinstruments/Discord-SFZ-GM-Bank@7a9c478fe331f94f246d33332f0adedb25bbbe27` | 49 | 10149165 | `88f111c3836110d0…` | `58423ce227b1` |
| 5 | `dsmolken-double-bass` (`dsmolken-double-bass`) | D. Smolken Rübner double bass | `CC0-1.0` | `sfzinstruments/dsmolken.double-bass@c2985eb647109d2a8f30a70071e3e163339d7396` | 406 | 288893197 | `31013f1b0522d7b1…` | `d27f0d8bd11a` |
| 6 | `freepats-button-accordion-hn` (`freepats-button-accordion-hn`) | FreePats Button Accordion HN | `CC0-1.0` | `freepats/button-accordion-HN@d70d16456fd99305d1c24c612b205ab38846eb0f` | 37 | 4470533 | `83db66fed7ffda96…` | `1f273b331061` |
| 7 | `freepats-drawbar-organ` (`freepats-drawbar-organ`) | FreePats Drawbar Organ Emulation | `CC0-1.0` | [archive](https://freepats.zenvoid.org/Organ/DrawbarOrganEmulation/DrawbarOrganEmulation-SFZ-20190712.tar.xz) | 18 | 6505808 | `162b026b2022a766…` | `d3fbbf3d9683` |
| 8 | `freepats-electric-bass-yr` (`freepats-electric-bass-yr`) | FreePats Electric Bass Guitar YR | `CC0-1.0` | `freepats/electric-bass-YR@8dcb7ea9116f417273ef8c030d15e7b3aa654301` | 29 | 6277261 | `ac56db99c722db45…` | `ba8c6ad0cd3c` |
| 9 | `freepats-fsbs-dist2` (`freepats-fsbs-dist2`) | FreePats FSBS Electric Guitar Distorted #2 | `CC0-1.0` | `freepats/electric-guitar-FSBS-dist2@21261b8bcb02d1cbf52dc02f1b1636df52d4a947` | 125 | 136921363 | `cf6ef0b9127692ca…` | `746b36690f1d` |
| 10 | `freepats-percussive-organ` (`freepats-percussive-organ`) | FreePats Percussive Organ Emulation | `CC0-1.0` | [archive](https://freepats.zenvoid.org/Organ/PercussiveOrganEmulation/PercussiveOrganEmulation-SFZ-20190715.tar.xz) | 34 | 14548534 | `b5dfd6639ab24d48…` | `ac09175af24d` |
| 11 | `freepats-spanish-classical-guitar` (`freepats-spanish-classical-guitar`) | FreePats Spanish Classical Guitar | `CC0-1.0` | `freepats/spanish-classical-guitar@6f4eb1b092acc88f5448cea1a0001bd07b971af8` | 51 | 5298000 | `b6a9c3fae9956e96…` | `7edec559c98c` |
| 12 | `ganjo` (`ganjo`) | Ganjo | `CC0-1.0` | `sfzinstruments/ganjo@ccff5cd5cd3b513873a48994c07724d9d3c39e1c` | 65 | 24493786 | `53303414a3ec946a…` | `9717cacbd1f1` |
| 13 | `hungarian-zither` (`hungarian-zither`) | Hungarian zither | `CC0-1.0` | `sfzinstruments/hungarian_zither@973d9445ba890661a4f4cd8e417d36134fb5f337` | 219 | 178322902 | `1f39a3559842f143…` | `0dc7cc2c070b` |
| 14 | `ixox-flute` (`ixox-flute`) | Ixox Flute | `CC-BY-4.0` | `sfzinstruments/Ixox.Flute@0cc54468bb0d2d9b32921958585caad65ba8df21` | 161 | 10215135 | `0ad20868c281b84f…` | `a26dd447a1f3` |
| 15 | `karoryfer-272-merry-orks` (`karoryfer-272-merry-orks`) | 272 Merry Orks | `CC0-1.0` | `sfzinstruments/karoryfer.272-merry-orks@a437e2c02014e02710a104a6692193eab8672d0a` | 277 | 45627769 | `1885678ca07e11e7…` | `68f034d7fa0f` |
| 16 | `karoryfer-bear-sax` (`karoryfer-bear-sax`) | Bear Sax | `CC0-1.0` | `sfzinstruments/karoryfer.bear-sax@7abb3c652525a15dfac80e1b5dfbba9964ee568f` | 814 | 143854044 | `4722c9c8929c251d…` | `7278ee6e4559` |
| 17 | `karoryfer-big-rusty-drums` (`karoryfer-big-rusty-drums`) | Big Rusty Drums | `CC0-1.0` | `sfzinstruments/karoryfer.big-rusty-drums@f07ce00df34a46b6b08375be56fe116cf15782bc` | 4814 | 706838139 | `0e354242ae7c3275…` | `369f9be584b6` |
| 18 | `karoryfer-bigcat-cello` (`karoryfer-bigcat-cello`) | Karoryfer × bigcat cello | `CC0-1.0` | `sfzinstruments/karoryfer-bigcat.cello@6fd75fbfc1dbb3109bf26220ba1adea46188a18b` | 520 | 141812275 | `9332d82becf44444…` | `e3eba9a133e4` |
| 19 | `karoryfer-black-and-blue-basses` (`karoryfer-black-and-blue-basses`) | Karoryfer Black And Blue Basses | `CC0-1.0` | `sfzinstruments/karoryfer.black-and-blue-basses@6e7d674cdb41be7a54dbccb15472401ad01099b9` | 2274 | 1124158166 | `e0469f26d5849b9b…` | `98ce9c4a206a` |
| 20 | `karoryfer-cowsynth` (`karoryfer-cowsynth`) | Cowsynth | `CC0-1.0` | `sfzinstruments/karoryfer.cowsynth@5a5b5afc2dabbe54cf9d75ab64711ce01862b42c` | 59 | 14832493 | `84372c620aff0f8d…` | `cf2c8abd9dcf` |
| 21 | `karoryfer-emilyguitar` (`karoryfer-emilyguitar`) | Karoryfer Emilyguitar | `CC0-1.0` | `sfzinstruments/karoryfer.emilyguitar@b4920dc662fd9cad6dcaccdeecffdd91c8725d8c` | 331 | 125538629 | `9c53a9a2e58e113b…` | `6374cae85165` |
| 22 | `karoryfer-meatbass` (`karoryfer-meatbass`) | Karoryfer Meatbass | `CC0-1.0` | `sfzinstruments/karoryfer.meatbass@ac9e859564bda286ab5ec672d00ff1aa2fef2895` | 587 | 296405063 | `fc3637a2ed63eecf…` | `55ea0ebe13b2` |
| 23 | `karoryfer-pastabass` (`karoryfer-pastabass`) | Karoryfer Pastabass (linguine) | `CC0-1.0` | `sfzinstruments/karoryfer.pastabass@90135cd026db5d4fa0fe538240b4203f085f5244` | 207 | 125954822 | `5dde6e39afffa9ff…` | `1faa0913f2b8` |
| 24 | `karoryfer-squidpipes` (`karoryfer-squidpipes`) | Squidpipes | `CC0-1.0` | `sfzinstruments/karoryfer.squidpipes@b258528c8f49d6389ec2b4ec04a8b10013169dd9` | 437 | 51873578 | `b200f736029f7364…` | `684780684388` |
| 25 | `karoryfer-string-cyborgs` (`karoryfer-string-cyborgs`) | String Cyborgs | `CC0-1.0` | `sfzinstruments/karoryfer.string-cyborgs@f2238b3e36ae64c6383356221dc89c9c476c7b11` | 304 | 74501792 | `4cacf3a31061c621…` | `6a07cdcc60bf` |
| 26 | `mtg-solo-sax` (`mtg-solo-sax`) | MTG Solo Saxophones | `CC-BY-4.0` | `sfzinstruments/MTG.SoloSax@b494d256549b3d088fdec176ce82867f8a1f58b2` | 784 | 110931713 | `9bbc1f9778a5404f…` | `21dcfa07a1dc` |
| 27 | `salamander-grand` (`salamander-grand`) | Salamander Grand Piano | `CC-BY-3.0` | `sfzinstruments/SalamanderGrandPiano@3382bf9496bba2486f5ab0de55a264d1dfc38404` | 667 | 748451231 | `1bad8bd293d3ece6…` | `c8b282f03fdb` |
| 28 | `vcsl` (`vcsl`) | VCSL — Versilian Community Sample Library | `CC0-1.0` | `sgossner/VCSL@dfcf4a4918771eee884b96ad4493de82ef84daf6` | 2651 | 2525491354 | `f099b618761214d2…` | `1074159ec33c` |
| 29 | `virtuosity-drums-basic` (`virtuosity-drums`) | Virtuosity Drums — Basic Kit | `CC0-1.0` | `sfzinstruments/virtuosity_drums@9f04cf9a7345` | 2078 | 443075501 | `1dcad38fa13e7d70…` | `a04683d87ad8` |
| 30 | `vsco2ce` (`vsco2ce`) | VSCO 2 CE — Versilian Studios Chamber Orchestra: Community Edition | `CC0-1.0` | `schollz/VSCO-2-CE@6dd651d55dde97fd4028699be9d4481f26917891` | 1897 | 2085134156 | `d3f7d060b1dac60a…` | `2a6111bc70bd` |

许可分布: `CC-BY-3.0` × 1, `CC-BY-4.0` × 2, `CC0-1.0` × 27。

### 3.1 需要署名的乐器 (CC-BY, `attribution_required: true`)

CC0 素材不要求署名, 以下 3 款 CC-BY 素材**必须**保留署名:

| 乐器 | 许可 | 署名对象 (author) | 来源 |
| :--- | :--- | :--- | :--- |
| Ixox Flute | `CC-BY-4.0` | Xavier Hosxe | https://github.com/sfzinstruments/Ixox.Flute |
| MTG Solo Saxophones | `CC-BY-4.0` | MTG (Music Technology Group, Universitat Pompeu Fabra); SFZ 转换 kinwie | https://github.com/sfzinstruments/MTG.SoloSax |
| Salamander Grand Piano | `CC-BY-3.0` | Alexander Holm | https://github.com/sfzinstruments/SalamanderGrandPiano |

许可版本的核实依据: `sfzinstruments/SalamanderGrandPiano` 的 README 写明 "Creative Commons
Attribution 3.0 Unported License"(作者 Alexander Holm); `sfzinstruments/MTG.SoloSax` 与
`sfzinstruments/Ixox.Flute` 的仓库许可证经 GitHub License API 核实为 `CC-BY-4.0`。

## 4. 被过滤掉的条目及原因 (Excluded)

| 乐器 | 上游许可 | 文件数 | 字节 | 是否允许商用 | 过滤原因 |
| :--- | :--- | ---: | ---: | :--- | :--- |
| `jlearman-jrhodes3c` — jRhodes3c — 1977 Rhodes Mark I Stage 73 | `CC-BY-NC-SA` | 136 | 17566470 | **否** | CC-BY-NC-SA: 含**非商业**限制 ⇒ 违反 AGENTS.md §2 红线 2 与 MUST-GATE-014 的 CC0/CC-BY/MIT 白名单, 不得入库。 |
| `jlearman-steel-drum` — jSteelDrum — C steel drum | `Unlicense` | 372 | 38541639 | 是 | Unlicense: 不在白名单内, 且 `ADR-0001 D20` 接受 Unlicense 是针对**代码依赖**的裁决, 不等于素材裁决 ⇒ **待人类负责人裁决**(见 needs N1)。 |
| `sonatina-brass` — Sonatina Symphonic Orchestra — Brass | `CC-Sampling-Plus` | 403 | 161561879 | 是 | CC-Sampling-Plus: 不在 MUST-GATE-014 白名单(CC0/CC-BY/MIT)内 ⇒ 不得入库, 即使它允许商用。 |

这三条**只出现在本文件与清单的 `excluded[]` 里, 绝不进 `items[]`** ——
`items[]` 是"允许入库"的唯一载体, 它的条目数就是 §0 里那个 30 的来源。

## 5. 怎么校验真实字节 (Fetch & Verify)

仓库里没有字节, 所以"校验"必须先拉取。完整可跑脚本(Bash + Python3, 本工作线实测通过)见
`docs/ledger/samples-attribution-notes.md` §校验脚本。要点:

```bash
# 1) 按 repo@pin 拉取某个乐器的整棵树(或按 items[].relative_path 逐个拉)
git clone --filter=blob:none --no-checkout https://github.com/<repo> /tmp/<prefix>
git -C /tmp/<prefix> fetch --depth 1 origin <pin> && git -C /tmp/<prefix> checkout FETCH_HEAD

# 2) 逐文件重算 sha256 并与清单对账(清单的 relative_path 口径 = 仓库根相对)
python3 - <<'PY'
import hashlib, json, pathlib
doc = json.load(open("assets/samples/manifest.json"))
for item in doc["items"]:
    p = pathlib.Path(item["relative_path"])
    if not p.is_file():
        continue
    actual = hashlib.sha256(p.read_bytes()).hexdigest()
    assert actual == item["sha256"], f"{p} 与清单不符"
    assert p.stat().st_size == item["size_bytes"], f"{p} 大小不符"
print("全部落地文件与清单一致")
PY

# 3) 两个 FreePats 风琴是 archive 溯源(非 git): 直接校验整包 sha256
curl -sSLO https://freepats.zenvoid.org/Organ/DrawbarOrganEmulation/DrawbarOrganEmulation-SFZ-20190712.tar.xz
shasum -a 256 DrawbarOrganEmulation-SFZ-20190712.tar.xz
# 期望 e2da18b0a4d13be7020037e18e4a719387433357e7603d0773990e794dcf5d0f
```

## 6. 已知缺口 (Known Gaps)

1. **许可白名单没有机械保护**。`scripts/gates/validate_schemas.py` **不读** `license`, 也不读根清单的
   `allowed_licenses`(全仓库无任何代码读它们)。实测: 往 `items[]` 注入一条 `license: CC-BY-NC-SA`
   的**结构合法**条目后, `--repo-assets` **依旧绿**。补这条判据必须改 `scripts/`(集成者独占)。
2. **30 ≠ 323**。差额 293 款没有素材来源, 需要人类负责人裁决(`HD-31`)后另行登记。
3. `Unlicense` 的 `jSteelDrum` **未被登记也未被判死**, 等人类负责人裁决(见 notes §needs N1)。

<!-- 清单 sha256: 39db1651ec6deb1b3831d8a69cac3155845a4b6e296be20ebebdbebe19fec84d -->

