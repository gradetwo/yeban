# `container` 工作线台账：`.yeban` 归档容器、两条 MUST 防御、判据与未决项

- **台账类型**：交付映射 / 格式实测 / 判据清单 / 未决项（**不是规范**）
- **工作线**：`line/container`（worktree `yeban/.worktrees/container`）
- **所有者目录**：`../../crates/yeban-model/**`（本台账是唯一新增的文档文件）
- **规范来源**：
  - [`../YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`](../YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md) §5.3 `[ARCH-SEC-003]`（ZIP 容器 + 两条 MUST）、§5.3 `[ARCH-SEC-004]`（原子落盘）、§5.2 `[ARCH-DET-001]`（写入确定性）
  - [`../YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`](../YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md) `[MUST-GATE-006]`（Zip-Slip）、`[MUST-GATE-007]`（解压炸弹）、§5.1 基准环境
  - **PKWARE APPNOTE.TXT 6.3.10**（`.ZIP` File Format Specification）—— ZIP 字节布局的唯一外部依据
  - [`../DEV_WORKFLOW.md`](../DEV_WORKFLOW.md) §2/§3、[`../DEVELOPMENT_LEDGER.md`](../DEVELOPMENT_LEDGER.md) L6 / L14 / L16 / L17 / L21

> 本文件回答五个问题：**我交付了什么对应哪条规范**、**我实现的 ZIP 子集到底长什么样（含第三方工具实测）**、
> **每条判据怎么变红**、**哪些东西明确没做**、**需要谁裁决什么**。

---

## 0. 一句话结论

`yeban-model` 现在有**零新增依赖、零 `unsafe`** 的 `.yeban` 容器读写器：只写标准 `stored` ZIP，
读到 `deflate` 明确报错；`MUST-GATE-006` 与 `MUST-GATE-007` 各自有**注入→变红**的记录（§5）。
**84 条新判据**全部在本机真跑（`bash scripts/gates/run-gates.sh crate yeban-model`，不是 SKIP）。

---

## 1. 落地清单（文件 ↔ 规范 ID）

| 文件 | 规范 ID | 说明 |
| :--- | :--- | :--- |
| [`../../crates/yeban-model/src/container/mod.rs`](../../crates/yeban-model/src/container/mod.rs) | `ARCH-SEC-003`、`MUST-GATE-006`、`MUST-GATE-007` | 公开 API：`ContainerLimits`（4 道可注入阈值）、`ContainerEntry`、`ContainerArchive`、`write_container` / `read_container`、§5.3 内容布局（`write_project_container` / `read_project_container`、`ProjectArchive`）、`ContainerError`（38 个变体，每个都能独立触发） |
| [`../../crates/yeban-model/src/container/zip.rs`](../../crates/yeban-model/src/container/zip.rs) | 同上 | 最小 ZIP 读写器（local header / central directory / EOCD；ZIP32 子集）；判定顺序即契约；`find_eocd` 尾部扫描；panic-free 游标 `Reader` |
| [`../../crates/yeban-model/src/container/path.rs`](../../crates/yeban-model/src/container/path.rs) | `MUST-GATE-006` | `normalize_entry_name`：纯语法层规范化判定（**接受 == 原样，拒绝 == 报错**，绝不静默重写） |
| [`../../crates/yeban-model/src/container/crc32.rs`](../../crates/yeban-model/src/container/crc32.rs) | `ARCH-SEC-003` | CRC-32/ISO-HDLC 查表实现（`const fn` 编译期建表，零初始化、零分配） |
| [`../../crates/yeban-model/src/lib.rs`](../../crates/yeban-model/src/lib.rs) | — | 一行 `pub mod container;` + 4 个再导出（本线对该文件只有这一处改动） |
| [`../../crates/yeban-model/tests/container_adversarial.rs`](../../crates/yeban-model/tests/container_adversarial.rs) | `MUST-GATE-006`、`MUST-GATE-007` | 对抗性判据 50 条（Zip-Slip 23 / 炸弹 9 / 结构畸形 18） |
| [`../../crates/yeban-model/tests/container_roundtrip.rs`](../../crates/yeban-model/tests/container_roundtrip.rs) | `ARCH-SEC-003`、`ARCH-DET-001` | 正向判据 16 条（往返、确定性、§5.3 布局、CAS 完整性、`proptest` 属性测试、`unzip` 互操作） |
| [`../../crates/yeban-model/tests/container_support/mod.rs`](../../crates/yeban-model/tests/container_support/mod.rs) | — | 共用夹具：写出合法容器 → **只改一个字段** → 得到恶意归档（保证变红可归因） |

**没有新增任何依赖。** `serde` / `serde_json` / `sha2` / `thiserror` / `ulid` 都是 `yeban-model` 已有的；
本线**明确拒绝**了引入 `zip` / `flate2`（见 §2 的裁决）。没有 `TODO(hoist)`。

**改动到的共享文件**：**无**。根 `Cargo.toml`、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`schemas/**`、其它 `crates/**`、`spikes/**`、法务文件全部未动。

---

## 2. 我实现的 ZIP 子集与依据

### 2.1 写：只写 `stored`，但产出的是标准 ZIP

| 字段 | 取值 | 依据/理由 |
| :--- | :--- | :--- |
| 压缩法 | `0` = `stored`（不压缩） | 资产池里是已压缩的音频（FLAC/WAV），deflate 收益很小 |
| DOS 时间戳 | `1980-01-01 00:00:00`（常量） | 确定性：时间不进入字节流 [`ARCH-DET-001`] |
| `version needed to extract` | `20`（2.0） | ZIP32 基线 |
| `version made by` | `0x031E`（Unix / 规范 3.0） | 让 `unzip` 认得 Unix 模式位 |
| 通用位标志 | `0x0800`（UTF-8 名字） | 名字是 UTF-8 |
| 外部属性 | `0o100644 << 16` | Unix 常规文件 |
| extra field / comment | 长度 `0` | 不写冗余结构 |

**好处（本线的设计目标）**：`yeban-model` 不新增依赖 ⇒ 全部判据（含对抗性归档）能在本机
`run-gates.sh crate yeban-model` **真编译真跑**，不必等 CI。这是"本机绿 ≠ CI 绿"这条老问题
在本线上的直接消灭方式。

### 2.2 读：不支持的东西一律**明确报错**，绝不静默跳过

`deflate`(8) / 其它压缩法 → `UnsupportedCompression`；ZIP64 → `UnsupportedZip64`；
加密 → `EncryptedEntryUnsupported`；data descriptor → `UnsupportedDataDescriptor`；
多卷 → `UnsupportedMultiDisk`；Unix 符号链接条目 → `SymlinkEntryUnsupported`；
非 UTF-8 名字 → `EntryNameNotUtf8`（CP437 不猜，见 §7）。

### 2.3 读路径的核心裁决：**central directory 是权威，且必须与 local header 一致**

ZIP 允许 local header 与 central directory 对同一字段（尤其是**名字**）给出不同值 ——
这是"不同解包器读出不同文件"的真实攻击面（历史上有解包器以 local 为准、有以 central 为准）。
本读取器**两处都读、逐字段比对、任何不一致直接拒绝**（`LocalCentralMismatch`）。
**歧义本身就是漏洞，消除歧义的方式是拒绝而不是"选一个"。** 判据：
`local_central_name_mismatch_is_rejected`、`local_central_size_mismatch_is_rejected`。

### 2.4 第三方工具实测（本机真跑，`unzip` 6.00 / Info-ZIP Apple 修改版）

命令与输出（归档由本实现写出，含 `project.json` / `history.dag` / `assets/{sha256}` 三条）：

```text
$ unzip -l /tmp/yeban-container-probe/interop.yeban
Archive:  interop.yeban
  Length      Date    Time    Name
---------  ---------- -----   ----
      664  01-01-1980 00:00   project.json
       25  01-01-1980 00:00   history.dag
     4096  01-01-1980 00:00   assets/78ee600cfd5be084b93300d3b9fc55505765cd440411ca0fb6ff974bf95a7e13
---------                     -------
     4785                     3 files
$ unzip -t /tmp/yeban-container-probe/interop.yeban
    testing: project.json             OK
    testing: history.dag              OK
    testing: assets/78ee600c...a7e13   OK
No errors detected in compressed data of interop.yeban.
$ unzip -p /tmp/yeban-container-probe/interop.yeban assets/78ee600c...a7e13 | shasum -a 256
78ee600cfd5be084b93300d3b9fc55505765cd440411ca0fb6ff974bf95a7e13  -
```

**最后一行是本线最强的正向证据**：`unzip` 解出的资产字节，其 SHA-256 **等于条目名里携带的 CAS 键** ——
内容寻址（CAS）由第三方工具端到端验证通过。

`zipinfo -v` 也确认了写入的字段取值：

```text
  file system or operating system of origin:      Unix
  version of encoding software:                   3.0
  minimum software version required to extract:   2.0
  compression method:                             none (stored)
  file security status:                           not encrypted
  extended local header:                          no
  file last modified on (DOS date/time):          1980 Jan 1 00:00:00
  compressed size:                                664 bytes
  uncompressed size:                              664 bytes
```

**确定性实测**：同一探针连续跑两次，归档 SHA-256 均为
`fc0586d1d5c9f520b97edd32aff1afad7227829edcaea56cb4280065a08d7992`（5223 字节）。

以上"手动"证据之外，判据 `unzip_reads_our_container` 把同一件事**自动化**了
（`unzip -l` / `unzip -t` / `unzip -p` 三步断言，本机实测通过；缺 `unzip` 的极小容器会**显式打印 skip** 并跳过，
不会伪装成通过）。

---

## 3. 两条 MUST 防御

### 3.1 `MUST-GATE-006` Zip-Slip

实现：`normalize_entry_name(&str) -> Result<String, ContainerError>`（纯函数，可单独测）。
**接受 == 原样，拒绝 == 报错**：不"修正"危险名字。因为把恶意条目重写成合法条目，
等于把攻击信号变成**看起来正常**的条目 —— 调用方再也无法区分"归档本来就是这样"和"我们改过它"。

判定清单（每条都有对应判据）：

| 构造 | 错误码 | 为什么 |
| :--- | :--- | :--- |
| 空名字 / 超过 4096 字节 | `EmptyEntryName` / `EntryNameTooLong` | 不是合法文件名 / 内存放大 |
| `/...`、`//server/share` | `AbsoluteEntryPath` | 绝对根路径、UNC |
| 含 `..` 段 | `ParentDirSegment` | Zip-Slip 本体 |
| 含 `.` 段 | `CurrentDirSegment` | 跨平台折叠规则不同 |
| 空段（`a//b`、`a/`） | `EmptyPathSegment` / `DirectoryEntryUnsupported` | 空段被平台折叠；`.yeban` 只存文件 |
| 含 `\` | `BackslashInEntryName` | Windows 上是分隔符 |
| 含 `:` | `ColonInEntryName` | `C:\`、`C:rel` 盘符、NTFS 数据流 |
| NUL / 其它控制字符 | `NulInEntryName` / `ControlCharInEntryName` | 截断与 shell 注入 |
| 段尾是 `.` 或空格 | `TrailingDotOrSpaceSegment` | **Windows 会剥掉它们** ⇒ `".. "` 等价于 `..` |
| Windows 设备名（`NUL`/`COM1`…） | `WindowsReservedName` | 写的是设备不是文件 |
| 大小写折叠重名 | `DuplicateEntryName` | APFS/NTFS 上互相覆盖 |
| Unix 符号链接条目 | `SymlinkEntryUnsupported` | 规范点名的"跨卷符号链接" |

**不做的事**：不对名字做百分号解码（ZIP 没有这一步，`..%2f` 是**字面文件名**，由判据正面钉住）、
不做 Unicode NFC/NFD 规范化（全角 `．` 是字面字符，由判据正面钉住）。
这两条是**有意的**：任何"先解码/先规范化再判定"的读者才会引入绕过。

### 3.2 `MUST-GATE-007` 解压炸弹

四道**可注入**上限（`ContainerLimits`），默认值即规范值：

| 字段 | 默认 | 规范 |
| :--- | :--- | :--- |
| `max_entry_bytes` | `2_000_000_000` | 单条目 ≤ 2 GB |
| `max_ratio` | `100` | 整体膨胀比率 ≤ 100:1 |
| `max_total_bytes` | `8_000_000_000` | 本实现额外加的一道（规范未钉） |
| `max_entries` | `4096` | 本实现额外加的一道（规范未钉） |

**为什么 `max_entry_bytes` 取 `2×10⁹` 而不是 2 GiB**：2 GiB 比 2 GB 宽 7.4%，
会在"GB 到底怎么算"的争议里站到放宽的一侧。安全上限一律往**紧**的一侧取（判据 `spec_defaults_are_pinned` 钉住）。

**"实际写入量"是防谎报声明的关键**：判定既看 ZIP 里**声明**的 `uncompressed_size`（fail-fast），
也看读取器**真的 materialize** 的字节数（兜底）。谎报 `uncompressed = 1` 的 4096 字节条目会在
`EntryActualTooLarge` 这一道被拦下（判据 `lying_declared_size_is_caught_by_actual_bytes`）。

### 3.3 判定顺序（**顺序本身是契约的一部分**）

```text
EOCD 定位 → 卷/条目数/ZIP64 哨兵 → central directory 边界+签名+逐条解析+长度核对
  → 逐条: 压缩法 → 加密/data descriptor → ZIP64 哨兵 → 路径规范化(006)
        → 符号链接 → 声明体积上限 → 实际体积上限 → 累计实际体积
        → 膨胀比率(007, fail-fast 用声明值) → stored 尺寸一致 → local/central 一致 → CRC
  → 大小写折叠重名
```

比率闸门排在"尺寸一致性"**之前**是刻意的：炸弹**声明**本身就足以拒绝，不该等结构核对完再拒绝；
而"实际体积"闸门用真实字节兜底。两者覆盖不同攻击（真炸弹 vs 假声明），判据分别钉住。

---

## 4. 判据清单：**84 条**（本机全部真跑）

| 位置 | 条数 | 覆盖 |
| :--- | :--- | :--- |
| `src/container/crc32.rs` 单元测试 | 5 | 标准向量（含 zlib 独立核对，见 §9）、单比特翻转全变、查表 vs 逐位参考 |
| `src/container/path.rs` 单元测试 | 8 | 三种容器形状接受、四类穿越、绝对/跨卷、空段、NUL/控制字符、尾随点空格、设备名、超长 |
| `src/container/zip.rs` 单元测试 | 5 | 比率边界精确、EOCD 尾部扫描（含注释、含更早的假签名）、写入 EOCD 布局、截断不 panic |
| `tests/container_adversarial.rs` | 50 | Zip-Slip 23 / 炸弹 9 / 结构畸形 18（见下） |
| `tests/container_roundtrip.rs` | 16 | 往返 5、§5.3 布局 9、`proptest` 属性 1、`unzip` 互操作 1 |
| **合计** | **84** | 基线为 89 条 lib 判据，本线后为 107 条 lib（+18）+ 66 条集成 |

**每条判据怎么变红**：全部判据断言的是**精确错误码**（`assert_eq!(err, ContainerError::X{..})`）
或**精确失败模式**（被接受/被拒绝），不是 `is_err()` 一类的弱断言。
因此：删掉/放宽/写错任何一条防御，对应判据必然变红；`ParentDirSegment` 与
`TrailingDotOrSpaceSegment` 这种"双覆盖"情形也会因为**错误码不同**而变红。
§5 的 7 次注入是这条规则的实测样本（不是所有 84 条都被逐个注入，见 §6 的边界声明）。

| 判据组 | 变红方式（实测/推理） |
| :--- | :--- |
| Zip-Slip 23 条 | 删任一条规则 ⇒ 错误码退化或归档被接受（§5 A/B） |
| 炸弹 9 条 | 删阈值比较 ⇒ 超限归档被接受（§5 C/G）；只信声明值 ⇒ 错误码退化（§5 D） |
| 结构畸形 18 条 | 删对应检查 ⇒ 恶意结构被接受（§5 E/F/G 覆盖 CRC / local-central / 条目数） |
| 往返 16 条 | 改写入器任一常量（时间戳/标志位/字段序）⇒ `write_is_deterministic` 与往返判据变红 |
| CRC 单元 5 条 | 改多项式/初值/异或值 ⇒ 标准向量变红（`0xCBF43926` 是公开检查值） |

---

## 5. 注入实验（**7 次，全部注入→变红→还原**）

方法：把 `src/container/**` 备份到 `/tmp/yeban-injection-backup/`，用定点改写删掉一条防御，
跑判据，记录输出，再从备份**逐字节还原**（§5.8 有还原校验）。
每次注入只改一处，且改动在判据里能对应到具体错误码。

### A. 删掉 `..` 段拦截（`path.rs` 的 `ParentDirSegment`）

```text
$ bash scripts/dev/cargo-local.sh test -p yeban-model --test container_adversarial
test bare_dotdot_name_is_rejected ... FAILED
test deep_dotdot_etc_passwd_is_rejected ... FAILED
test dotdot_relative_path_is_rejected ... FAILED
test interior_dotdot_segment_is_rejected ... FAILED
test write_container_rejects_unsafe_names ... FAILED
  left: TrailingDotOrSpaceSegment { name: "../evil" }
 right: ParentDirSegment { name: "../evil" }
test result: FAILED. 45 passed; 5 failed
```

**结论**：5 条变红。注意错误码**退化**成 `TrailingDotOrSpaceSegment` —— 因为 `..` 同时被
"段尾是点"这条规则覆盖。这是**有意**的双覆盖（纵深防御），判据因为断言精确错误码而仍然可观测，
但这也说明：单靠段尾规则也能拦住经典 `..`（见 B 的补证）。

### B. 连"段尾是点/空格"规则一起删掉（A 仍生效）

```text
test bare_dotdot_name_is_rejected ... FAILED
test deep_dotdot_etc_passwd_is_rejected ... FAILED
test dotdot_relative_path_is_rejected ... FAILED
test interior_dotdot_segment_is_rejected ... FAILED
test windows_trailing_dot_or_space_bypass_is_rejected ... FAILED
test write_container_rejects_unsafe_names ... FAILED
期望被拒绝，实际读出了 1 个条目   （×5）
test result: FAILED. 44 passed; 6 failed
```

**结论**：`../evil`、`../../etc/passwd`、`a/../../b`、`..` 全部**被接受** —— 这是本线最强的
falsification：`MUST-GATE-006` 的判据确实由这两条规则承载，不存在"判据自己绿自己"。

### C. 把比率闸门改成恒不超限（`ratio_exceeded` 直接 `return false`）

```text
$ bash scripts/dev/cargo-local.sh test -p yeban-model --test container_adversarial
test expansion_ratio_boundary_is_exactly_one_hundred ... FAILED
test expansion_ratio_over_limit_is_rejected ... FAILED
  left: StoredSizeMismatch { index: 0, name: "aaaa", compressed: 8, uncompressed: 801 }
 right: ExpansionRatioExceeded { uncompressed: 801, compressed: 8, max_ratio: 100 }
$ bash scripts/dev/cargo-local.sh test -p yeban-model --lib ratio_boundary
test container::zip::tests::ratio_boundary_is_exact ... FAILED
```

**结论**：集成判据 2 条 + 单元判据 1 条变红。`MUST-GATE-007` 的比率分量确实可观测。

### D. 只信声明值（删掉"实际体积"闸门 `EntryActualTooLarge`）

```text
test lying_declared_size_is_caught_by_actual_bytes ... FAILED
  left: StoredSizeMismatch { index: 0, name: "aaaa", compressed: 4096, uncompressed: 1 }
 right: EntryActualTooLarge { actual: 4096, max: 1024 }
```

**结论**：谎报声明的归档不再触发"实际体积"闸门，而是退到"stored 尺寸不一致"。
归档**依然被拒绝**（纵深防御有效），但**精确错误码判据变红** —— 这正是"按实际写入量再判一次"
那一行代码的承重性证明。见 §7 的诚实说明：在 `stored`-only 的子集里这两道闸门对"谎报声明"
有部分重叠；"实际体积"闸门的独立价值在 deflate 落地后（那时 `stored` 尺寸一致性不再约束压缩后大小）。

### E. 删掉 CRC 校验

```text
test crc_mismatch_is_rejected ... FAILED
期望 CrcMismatch，实际 Ok(ContainerArchive { entries: [ContainerEntry { name: "aaaa", data: [254, 2, 3, 4] }] })
```

**结论**：被篡改的字节（`1` → `254`）**静默读出**。

### F. 删掉 local header ↔ central directory 一致性比对

```text
test local_central_name_mismatch_is_rejected ... FAILED
test local_central_size_mismatch_is_rejected ... FAILED
期望被拒绝，实际读出了 1 个条目   （×2）
```

**结论**："名字以 local 为准还是以 central 为准"重新变成歧义 —— ZIP 的经典攻击面回归。

### G. 删掉条目数上限

```text
test entry_count_over_limit_is_rejected ... FAILED
test limits_are_injectable_on_tiny_data ... FAILED
期望被拒绝，实际读出了 5 个条目
期望被拒绝，实际读出了 1 个条目
```

### 5.8 还原校验

```text
$ diff -r crates/yeban-model/src/container /tmp/yeban-injection-backup/container && echo IDENTICAL
IDENTICAL
$ shasum -a 256 crates/yeban-model/src/container/*.rs
c97e5545…  crc32.rs      3a8a7276…  mod.rs
fe6a67c1…  path.rs       1b5cb45c…  zip.rs
$ bash scripts/gates/run-gates.sh crate yeban-model   →  门禁通过 (mode=crate)
```

4 个文件的 SHA-256 与注入前完全一致，门禁恢复全绿（107 + 50 + 16 = 173 条通过）。

---

## 6. 本机真跑 vs 交给 CI

| 项目 | 本机 | CI |
| :--- | :--- | :--- |
| `run-gates.sh crate yeban-model`（fmt + 13 守卫 + 文档链接 + 许可清单 + clippy `-D warnings` + test） | **真跑，绿** | 同样跑（不是 SKIP：`yeban-model` 不含 `zip`/`flate2`/`slint`/… 等重依赖） |
| 84 条容器判据（含 66 条集成的对抗性/往返判据） | **真跑，绿** | 同样跑 |
| `unzip` 互操作 | **真跑，绿**（`unzip` 6.00 本机） | 跑（Ubuntu 一般自带 `unzip`；缺失时判据显式 skip） |
| `cargo deny`、workspace 全量 clippy/test、跨架构 | 不跑（本机纪律） | 唯一的"绿"来源（`MUST-GATE-004` 等） |

**边界（这次没有证明什么）**：
1. 本机绿**不等于** CI 绿；CI 判决见 §10。
2. 84 条判据里**逐条注入**的只有 §5 的 7 次，其余 77 条是"reasoned falsifiability"（§4 的表），
   不是实测过的注入样本。**没有**做模糊测试（`cargo-fuzz` 属于 CI，本机纪律禁止）。
3. 没有做 2 GB 级别的**真实**大归档（判据用"声明 2 GB + 实际 8 字节"来钉上限本身，
   真正的大文件往返只测到 3 MiB）。
4. 没有测磁盘/文件系统层的原子性 —— 那是 `ARCH-SEC-004`（`yeban-mcp/store.rs`）的范围。

---

## 7. 未实现项 / pending / needs

**明确未实现（读取路径上**明确报错**，不是静默跳过）**：

| 项 | 状态 | 说明 |
| :--- | :--- | :--- |
| `deflate`(method 8) 及一切非 `stored` 压缩法 | 拒绝（`UnsupportedCompression`） | 本线裁决：不引入 `flate2`。将来要支持时必须**同时**补"压缩后大小的实际量"判据 |
| ZIP64 | 拒绝（`UnsupportedZip64`） | 单条目上限 2 GB ⇒ ZIP32 足够；EOCD/central 里的 `0xFFFF`/`0xFFFFFFFF` 哨兵一律拒绝 |
| 加密 / 密码 | 拒绝（`EncryptedEntryUnsupported`） | `.yeban` 的机密性不在本线范围 |
| data descriptor | 拒绝（`UnsupportedDataDescriptor`） | 本写入器永远先写尺寸 |
| 多卷 / 跨盘 | 拒绝（`UnsupportedMultiDisk`） | 归档必须是单文件 |
| CP437 名字（未置 UTF-8 标志的旧归档） | 拒绝（`EntryNameNotUtf8`） | 不做代码页猜测；本写入器始终置 UTF-8 标志 |

**pending（记录了但没做，不假装已覆盖）**：

1. **Unicode 规范化折叠**：`docs/YEBAN_…` 未要求，本实现也不做 NFC/NFD 归一。
   在 APFS（HFS+ 遗留）上，"两个不同码位"可能折叠成同一个文件名 ⇒ 理论上存在"重名覆盖"通道，
   与大小写折叠同族。补它需要 `unicode-normalization`（违反零依赖目标）或一份自研表。
   **登记为 pending，由集成者裁决**。
2. **Windows 上标数字设备名别名**（`COM¹` ≡ `COM1`）：`.yeban` 写入器只产出 ASCII 名字，
   读取侧的设备名检查不覆盖上标别名。**pending**。
3. **压缩后大小的"实际"读数**：`stored` 下实际字节数 == 数据区长度 == `compressed_size`，
   因此在当前子集里"实际体积"闸门与"stored 尺寸一致"闸门对谎报声明**有部分重叠**（§5 D）。
   真正的独立价值要等 deflate 落地。**pending（deflate 落地时必须补）**。
4. **重复名字的规范化折叠**：目前只折叠 ASCII 大小写（见 pending 1）。

**needs（需要别人做的事）**：

1. **集成者**：把本线的裁决写进 ADR（"不引入 `zip`/`flate2`，手写最小 ZIP 子集"），
   并在 `docs/DEVELOPMENT_LEDGER.md` 登记 `MUST-GATE-006` / `MUST-GATE-007` 的状态切换。
2. **集成者**：`crates/yeban-mcp/src/domain/store.rs`（`ARCH-SEC-004` 原子落盘）目前**不调用**本模块。
   把 `.yeban` 的保存/加载接到 `write_project_container` / `read_project_container` 上，
   是下一个能力切片（本线只交付字节层，不碰文件系统）。
3. **人**：`docs/ledger/container-notes.md` 是否需要登记进 `docs/README.md` 的账本索引
   （现有账本也未全部登记，本线未擅自改动该共享文件）。

**TODO(hoist)**：**无**。没有新增依赖，因此不需要把任何版本提升到根 `[workspace.dependencies]`。

---

## 8. 与 `[ARCH-SEC-004]`（原子落盘）的关系

| 层 | 责任 | 实现 |
| :--- | :--- | :--- |
| **字节层**（本线） | 字节流本身不可越界：路径、体积、比率、结构、CRC | `crates/yeban-model/src/container/**` |
| **I/O 层**（别人的线） | 落盘那一刻不可撕裂：`.yeban.tmp-{ulid}` + `File::sync_all()` + 原子 `rename` | [`../../crates/yeban-mcp/src/domain/store.rs`](../../crates/yeban-mcp/src/domain/store.rs) |

本线**不重复做** `ARCH-SEC-004`：容器模块是纯函数，输入 `&[u8]`、输出 `Vec<u8>`，
不碰文件系统、不做权限判定、不开文件。两者拼起来才是"安全保存 + 安全加载"。

---

## 9. CRC-32 的独立核对

CRC 表与向量没有手算：期望值用**独立实现**（Python 3 标准库 `zlib.crc32`，底层为 zlib）当场算出：

```text
empty      0x00000000      a          0xE8B7BE43      abc        0x352441C2
123456789  0xCBF43926      fox        0x414FA339      0x00       0xD202EF8D
0xFF       0xFF000000      32×0x00    0x190A55AD      32×0xFF    0xFF6CAB0B
```

（第一次写这条判据时我**抄错**了 `0xFF` 的向量，写成 `0xFF6CAB8D`，判据当场变红 ——
这恰好是"判据要跑过才算数"的现场证词；正确值 `0xFF000000` 由 `zlib` 核对。）

---

## 10. CI 判决

- 分支：`line/container`
- 判决：**见下方回填**（本文件先于推送写成，推送后由 `scripts/dev/ci-verdict.sh line/container` 读回）

<!-- CI-VERDICT -->

---

## 11. 本线修改/新增文件的绝对路径清单

```text
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/src/container/crc32.rs
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/src/container/mod.rs
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/src/container/path.rs
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/src/container/zip.rs
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/src/lib.rs          (修改：+1 模块声明 +1 再导出)
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/tests/container_adversarial.rs
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/tests/container_roundtrip.rs
/Users/crow/work/music/yeban/.worktrees/container/crates/yeban-model/tests/container_support/mod.rs
/Users/crow/work/music/yeban/.worktrees/container/docs/ledger/container-notes.md          (本文件)
```
