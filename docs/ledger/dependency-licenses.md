# 依赖许可清单（机器生成，请勿手工编辑）

> 由 `python3 scripts/gates/license_inventory.py` 从 `cargo metadata --locked` 生成。
> CI 用 `--check` 对账：加依赖/升版本后忘记重新生成，这一项会变红。

- `Cargo.lock` SHA-256（前 16 位）: `0ee9a063949bd44e`
- 外部依赖包数: **62**（不含 23 个 workspace 成员）
- 许可来源: 各包 `Cargo.toml` 的 `license` 字段（SPDX 表达式，未经人工改写）

## 许可族分布

| SPDX 表达式 | 包数 |
| :--- | ---: |
| `MIT OR Apache-2.0` | 47 |
| `MIT` | 4 |
| `Apache-2.0 OR MIT` | 3 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 2 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 1 |
| `Unlicense OR MIT` | 1 |

> 多许可表达式（`A OR B`）只要有一个分支在白名单内即通过 `cargo deny`；`AND` 则要求每一侧都被允许。`deny.toml` 的 `allow` 列表是唯一策略来源。

## 全量清单

| 包 | 版本 | 许可 | 类型 | 直接依赖它的成员 |
| :--- | :--- | :--- | :--- | :--- |
| `autocfg` | `1.5.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `bitflags` | `2.13.2` | `MIT OR Apache-2.0` | 传递 | — |
| `block-buffer` | `0.12.1` | `MIT OR Apache-2.0` | 传递 | — |
| `bumpalo` | `3.20.3` | `MIT OR Apache-2.0` | 传递 | — |
| `cfg-if` | `1.0.5` | `MIT OR Apache-2.0` | 传递 | — |
| `chacha20` | `0.10.2` | `MIT OR Apache-2.0` | 传递 | — |
| `const-oid` | `0.10.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `cpufeatures` | `0.3.1` | `MIT OR Apache-2.0` | 传递 | — |
| `crypto-common` | `0.2.2` | `MIT OR Apache-2.0` | 传递 | — |
| `digest` | `0.11.3` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-core` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-task` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-util` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `getrandom` | `0.3.4` | `MIT OR Apache-2.0` | 传递 | — |
| `getrandom` | `0.4.3` | `MIT OR Apache-2.0` | 传递 | — |
| `hybrid-array` | `0.4.15` | `MIT OR Apache-2.0` | 传递 | — |
| `itoa` | `1.0.18` | `MIT OR Apache-2.0` | 传递 | — |
| `js-sys` | `0.3.106` | `MIT OR Apache-2.0` | 传递 | — |
| `libc` | `0.2.190` | `MIT OR Apache-2.0` | 传递 | — |
| `libm` | `0.2.16` | `MIT` | 直接 | `yeban-theory` |
| `memchr` | `2.8.3` | `Unlicense OR MIT` | 传递 | — |
| `num-traits` | `0.2.19` | `MIT OR Apache-2.0` | 传递 | — |
| `once_cell` | `1.21.4` | `MIT OR Apache-2.0` | 传递 | — |
| `pin-project-lite` | `0.2.17` | `Apache-2.0 OR MIT` | 传递 | — |
| `ppv-lite86` | `0.2.21` | `MIT OR Apache-2.0` | 传递 | — |
| `proc-macro2` | `1.0.107` | `MIT OR Apache-2.0` | 传递 | — |
| `proptest` | `1.11.0` | `MIT OR Apache-2.0` | 直接 | `yeban-model`, `yeban-theory` |
| `quote` | `1.0.47` | `MIT OR Apache-2.0` | 传递 | — |
| `r-efi` | `5.3.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 传递 | — |
| `r-efi` | `6.0.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 传递 | — |
| `rand` | `0.10.3` | `MIT OR Apache-2.0` | 传递 | — |
| `rand` | `0.9.5` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_chacha` | `0.9.0` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_core` | `0.10.1` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_core` | `0.9.5` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_xorshift` | `0.4.0` | `MIT OR Apache-2.0` | 传递 | — |
| `regex-syntax` | `0.8.11` | `MIT OR Apache-2.0` | 传递 | — |
| `rustversion` | `1.0.23` | `MIT OR Apache-2.0` | 传递 | — |
| `serde` | `1.0.229` | `MIT OR Apache-2.0` | 直接 | `yeban-model` |
| `serde_core` | `1.0.229` | `MIT OR Apache-2.0` | 传递 | — |
| `serde_derive` | `1.0.229` | `MIT OR Apache-2.0` | 传递 | — |
| `serde_json` | `1.0.151` | `MIT OR Apache-2.0` | 直接 | `yeban-model` |
| `sha2` | `0.11.0` | `MIT OR Apache-2.0` | 直接 | `yeban-model` |
| `slab` | `0.4.12` | `MIT` | 传递 | — |
| `syn` | `2.0.119` | `MIT OR Apache-2.0` | 传递 | — |
| `syn` | `3.0.6` | `MIT OR Apache-2.0` | 传递 | — |
| `thiserror` | `2.0.21` | `MIT OR Apache-2.0` | 直接 | `yeban-model`, `yeban-theory` |
| `thiserror-impl` | `2.0.21` | `MIT OR Apache-2.0` | 传递 | — |
| `typenum` | `1.20.1` | `MIT OR Apache-2.0` | 传递 | — |
| `ulid` | `3.0.0` | `MIT` | 直接 | `yeban-model` |
| `unarray` | `0.1.4` | `MIT OR Apache-2.0` | 传递 | — |
| `unicode-ident` | `1.0.26` | `(MIT OR Apache-2.0) AND Unicode-3.0` | 传递 | — |
| `wasip2` | `1.0.1+wasi-0.2.4` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `wasm-bindgen` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-macro` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-macro-support` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-shared` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `web-time` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `wit-bindgen` | `0.46.0` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `zerocopy` | `0.8.59` | `BSD-2-Clause OR Apache-2.0 OR MIT` | 传递 | — |
| `zerocopy-derive` | `0.8.59` | `BSD-2-Clause OR Apache-2.0 OR MIT` | 传递 | — |
| `zmij` | `1.0.23` | `MIT` | 传递 | — |

## 源码级移植（不由 cargo 依赖图覆盖）

从**历史项目源码**改写移植进来的代码不在 `cargo deny` 视野内，归属与许可全文必须由 `THIRD_PARTY_LICENSES.md` 承载。当前登记：

- `synth-core`（MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors）—
  `crates/yeban-dsp` 的部分模块；逐文件裁决见 `docs/ledger/dsp-core-provenance.md`。
