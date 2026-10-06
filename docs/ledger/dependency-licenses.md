# 依赖许可清单（机器生成，请勿手工编辑）

> 由 `python3 scripts/gates/license_inventory.py` 从 `cargo metadata --locked` 生成。
> CI 用 `--check` 对账：加依赖/升版本后忘记重新生成，这一项会变红。

- `Cargo.lock` SHA-256（前 16 位）: `308834d23ff2009f`
- 外部依赖包数: **632**（不含 25 个 workspace 成员）
- 许可来源: 各包 `Cargo.toml` 的 `license` 字段（SPDX 表达式，未经人工改写）

## 许可族分布

| SPDX 表达式 | 包数 |
| :--- | ---: |
| `MIT OR Apache-2.0` | 293 |
| `MIT` | 130 |
| `Apache-2.0 OR MIT` | 52 |
| `Unicode-3.0` | 27 |
| `Apache-2.0` | 18 |
| `Zlib OR Apache-2.0 OR MIT` | 16 |
| `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 14 |
| `MIT/Apache-2.0` | 14 |
| `MPL-2.0` | 10 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 8 |
| `MIT OR Apache-2.0 OR Zlib` | 7 |
| `BSD-3-Clause` | 6 |
| `Unlicense OR MIT` | 4 |
| `Zlib` | 4 |
| `Apache-2.0/MIT` | 3 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 |
| `BSD-3-Clause OR Apache-2.0` | 2 |
| `BSD-3-Clause OR MIT OR Apache-2.0` | 2 |
| `BSL-1.0` | 2 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 2 |
| `MIT OR Zlib OR Apache-2.0` | 2 |
| `Unlicense/MIT` | 2 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 1 |
| `0BSD OR Apache-2.0` | 1 |
| `0BSD OR MIT OR Apache-2.0` | 1 |
| `Apache-2.0 / MIT` | 1 |
| `Apache-2.0 AND MIT` | 1 |
| `BSD-2-Clause` | 1 |
| `BSD-2-Clause OR MIT OR Apache-2.0` | 1 |
| `CC0-1.0 OR Apache-2.0` | 1 |
| `ISC` | 1 |
| `MIT / Apache-2.0` | 1 |
| `Unlicense` | 1 |
| `Zlib OR MIT OR Apache-2.0` | 1 |

> 多许可表达式（`A OR B`）只要有一个分支在白名单内即通过 `cargo deny`；`AND` 则要求每一侧都被允许。`deny.toml` 的 `allow` 列表是唯一策略来源。

## 全量清单

| 包 | 版本 | 许可 | 类型 | 直接依赖它的成员 |
| :--- | :--- | :--- | :--- | :--- |
| `ab_glyph` | `0.2.32` | `Apache-2.0` | 传递 | — |
| `ab_glyph_rasterizer` | `0.1.10` | `Apache-2.0` | 传递 | — |
| `accesskit` | `0.24.1` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_atspi_common` | `0.19.1` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_consumer` | `0.38.0` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_ios` | `0.1.2` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_macos` | `0.26.3` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_unix` | `0.22.1` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_windows` | `0.34.0` | `MIT OR Apache-2.0` | 传递 | — |
| `accesskit_winit` | `0.33.2` | `Apache-2.0` | 传递 | — |
| `adler2` | `2.0.1` | `0BSD OR MIT OR Apache-2.0` | 传递 | — |
| `ahash` | `0.8.12` | `MIT OR Apache-2.0` | 传递 | — |
| `aho-corasick` | `1.1.5` | `Unlicense OR MIT` | 传递 | — |
| `allocator-api2` | `0.2.21` | `MIT OR Apache-2.0` | 传递 | — |
| `alsa` | `0.11.0` | `Apache-2.0/MIT` | 传递 | — |
| `alsa-sys` | `0.4.0` | `MIT` | 传递 | — |
| `android-activity` | `0.6.1` | `MIT OR Apache-2.0` | 传递 | — |
| `android-properties` | `0.2.2` | `MIT` | 传递 | — |
| `android_system_properties` | `0.1.6` | `MIT OR Apache-2.0` | 传递 | — |
| `annotate-snippets` | `0.12.16` | `MIT OR Apache-2.0` | 传递 | — |
| `anstyle` | `1.0.14` | `MIT OR Apache-2.0` | 传递 | — |
| `arboard` | `3.6.1` | `MIT OR Apache-2.0` | 传递 | — |
| `arrayref` | `0.3.9` | `BSD-2-Clause` | 传递 | — |
| `arrayvec` | `0.7.8` | `MIT OR Apache-2.0` | 传递 | — |
| `as-raw-xcb-connection` | `1.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `ash` | `0.38.0+1.3.281` | `MIT OR Apache-2.0` | 传递 | — |
| `async-broadcast` | `0.7.2` | `MIT OR Apache-2.0` | 传递 | — |
| `async-channel` | `2.5.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-executor` | `1.14.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-io` | `2.6.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-lock` | `3.4.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-process` | `2.5.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-recursion` | `1.2.0` | `MIT OR Apache-2.0` | 传递 | — |
| `async-signal` | `0.2.14` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-task` | `4.7.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `async-trait` | `0.1.92` | `MIT OR Apache-2.0` | 传递 | — |
| `atomic-waker` | `1.1.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `atspi` | `0.29.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `atspi-common` | `0.13.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `atspi-proxies` | `0.13.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `audio-codec-algorithms` | `0.8.1` | `0BSD OR Apache-2.0` | 传递 | — |
| `audioadapter` | `5.0.0` | `MIT OR Apache-2.0` | 传递 | — |
| `audioadapter-buffers` | `5.2.0` | `MIT OR Apache-2.0` | 传递 | — |
| `audioadapter-sample` | `5.2.0` | `MIT OR Apache-2.0` | 传递 | — |
| `autocfg` | `1.5.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `base64` | `0.23.1` | `MIT OR Apache-2.0` | 传递 | — |
| `bincode` | `2.0.1` | `MIT` | 传递 | — |
| `bindgen` | `0.70.1` | `BSD-3-Clause` | 传递 | — |
| `bindgen` | `0.72.1` | `BSD-3-Clause` | 传递 | — |
| `bit-set` | `0.10.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `bit-vec` | `0.9.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `bitflags` | `1.3.2` | `MIT/Apache-2.0` | 传递 | — |
| `bitflags` | `2.13.2` | `MIT OR Apache-2.0` | 传递 | — |
| `block-buffer` | `0.12.1` | `MIT OR Apache-2.0` | 传递 | — |
| `block2` | `0.5.1` | `MIT` | 传递 | — |
| `block2` | `0.6.2` | `MIT` | 传递 | — |
| `blocking` | `1.7.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `borsh` | `1.8.1` | `MIT OR Apache-2.0` | 传递 | — |
| `bumpalo` | `3.20.3` | `MIT OR Apache-2.0` | 传递 | — |
| `by_address` | `1.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `bytemuck` | `1.25.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `bytemuck_derive` | `1.12.1` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `byteorder-lite` | `0.1.0` | `Unlicense OR MIT` | 传递 | — |
| `bytes` | `1.12.1` | `MIT` | 传递 | — |
| `calloop` | `0.13.0` | `MIT` | 传递 | — |
| `calloop` | `0.14.5` | `MIT` | 传递 | — |
| `calloop-wayland-source` | `0.3.0` | `MIT` | 传递 | — |
| `cc` | `1.6.0` | `MIT OR Apache-2.0` | 传递 | — |
| `cexpr` | `0.6.0` | `Apache-2.0/MIT` | 传递 | — |
| `cfg-if` | `1.0.5` | `MIT OR Apache-2.0` | 传递 | — |
| `cfg_aliases` | `0.2.2` | `MIT` | 传递 | — |
| `cgl` | `0.3.2` | `MIT / Apache-2.0` | 传递 | — |
| `chacha20` | `0.10.2` | `MIT OR Apache-2.0` | 传递 | — |
| `chrono` | `0.4.45` | `MIT OR Apache-2.0` | 传递 | — |
| `clang-sys` | `1.9.1` | `Apache-2.0` | 传递 | — |
| `clipboard-win` | `5.4.1` | `BSL-1.0` | 传递 | — |
| `clru` | `0.6.3` | `MIT` | 传递 | — |
| `codespan-reporting` | `0.13.1` | `Apache-2.0` | 传递 | — |
| `color_quant` | `1.1.0` | `MIT` | 传递 | — |
| `combine` | `4.6.8` | `MIT` | 传递 | — |
| `concurrent-queue` | `2.5.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `const-field-offset` | `0.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `const-field-offset-macro` | `0.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `const-oid` | `0.10.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `convert_case` | `0.10.0` | `MIT` | 传递 | — |
| `core-foundation` | `0.9.4` | `MIT OR Apache-2.0` | 传递 | — |
| `core-foundation-sys` | `0.8.7` | `MIT OR Apache-2.0` | 传递 | — |
| `core-graphics` | `0.23.2` | `MIT OR Apache-2.0` | 传递 | — |
| `core-graphics-types` | `0.1.3` | `MIT OR Apache-2.0` | 传递 | — |
| `coreaudio-rs` | `0.14.2` | `MIT/Apache-2.0` | 传递 | — |
| `countme` | `3.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `cpal` | `0.18.2` | `Apache-2.0` | 直接 | `yeban-engine` |
| `cpufeatures` | `0.3.1` | `MIT OR Apache-2.0` | 传递 | — |
| `crc32fast` | `1.5.2` | `MIT OR Apache-2.0` | 传递 | — |
| `critical-section` | `1.2.0` | `MIT OR Apache-2.0` | 传递 | — |
| `crossbeam-channel` | `0.5.17` | `MIT OR Apache-2.0` | 传递 | — |
| `crossbeam-deque` | `0.8.8` | `MIT OR Apache-2.0` | 传递 | — |
| `crossbeam-epoch` | `0.9.21` | `MIT OR Apache-2.0` | 传递 | — |
| `crossbeam-utils` | `0.8.23` | `MIT OR Apache-2.0` | 传递 | — |
| `crunchy` | `0.2.4` | `MIT` | 传递 | — |
| `crypto-common` | `0.2.2` | `MIT OR Apache-2.0` | 传递 | — |
| `ctor` | `0.10.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `cursor-icon` | `1.2.0` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `dasp` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_envelope` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_frame` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_interpolate` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_peak` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_ring_buffer` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_rms` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_sample` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_signal` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_slice` | `0.11.0` | `MIT OR Apache-2.0` | 传递 | — |
| `dasp_window` | `0.11.1` | `MIT OR Apache-2.0` | 传递 | — |
| `data-url` | `0.3.2` | `MIT OR Apache-2.0` | 传递 | — |
| `derive_more` | `2.1.1` | `MIT` | 传递 | — |
| `derive_more-impl` | `2.1.1` | `MIT` | 传递 | — |
| `digest` | `0.11.3` | `MIT OR Apache-2.0` | 传递 | — |
| `dispatch` | `0.2.0` | `MIT` | 传递 | — |
| `dispatch2` | `0.3.1` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `displaydoc` | `0.2.7` | `MIT OR Apache-2.0` | 传递 | — |
| `dlib` | `0.5.3` | `MIT` | 传递 | — |
| `document-features` | `0.2.12` | `MIT OR Apache-2.0` | 传递 | — |
| `downcast-rs` | `1.2.1` | `MIT/Apache-2.0` | 传递 | — |
| `dpi` | `0.1.2` | `Apache-2.0 AND MIT` | 传递 | — |
| `drm` | `0.14.1` | `MIT` | 传递 | — |
| `drm-ffi` | `0.9.1` | `MIT` | 传递 | — |
| `drm-fourcc` | `2.2.0` | `MIT` | 传递 | — |
| `drm-sys` | `0.8.1` | `MIT` | 传递 | — |
| `dtor` | `0.8.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `either` | `1.18.0` | `MIT OR Apache-2.0` | 传递 | — |
| `endi` | `1.1.1` | `MIT` | 传递 | — |
| `enumflags2` | `0.7.12` | `MIT OR Apache-2.0` | 传递 | — |
| `enumflags2_derive` | `0.7.12` | `MIT OR Apache-2.0` | 传递 | — |
| `equivalent` | `1.0.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `errno` | `0.3.14` | `MIT OR Apache-2.0` | 传递 | — |
| `error-code` | `3.4.0` | `BSL-1.0` | 传递 | — |
| `euclid` | `0.22.14` | `MIT OR Apache-2.0` | 传递 | — |
| `event-listener` | `5.4.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `event-listener-strategy` | `0.5.4` | `Apache-2.0 OR MIT` | 传递 | — |
| `extended` | `0.1.0` | `MIT` | 传递 | — |
| `fastrand` | `2.5.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `fdeflate` | `0.3.7` | `MIT OR Apache-2.0` | 传递 | — |
| `femtovg` | `0.27.0` | `MIT OR Apache-2.0` | 传递 | — |
| `field-offset` | `0.3.6` | `MIT OR Apache-2.0` | 传递 | — |
| `filetime` | `0.2.29` | `MIT/Apache-2.0` | 传递 | — |
| `find-msvc-tools` | `0.1.14` | `MIT OR Apache-2.0` | 传递 | — |
| `fixed_decimal` | `0.7.2` | `Unicode-3.0` | 传递 | — |
| `fixedbitset` | `0.5.7` | `MIT OR Apache-2.0` | 传递 | — |
| `flate2` | `1.1.10` | `MIT OR Apache-2.0` | 直接 | `yeban-render` |
| `float-cmp` | `0.9.0` | `MIT` | 传递 | — |
| `fnv` | `1.0.7` | `Apache-2.0 / MIT` | 传递 | — |
| `foldhash` | `0.1.5` | `Zlib` | 传递 | — |
| `foldhash` | `0.2.0` | `Zlib` | 传递 | — |
| `font-types` | `0.12.6` | `MIT OR Apache-2.0` | 传递 | — |
| `fontdb` | `0.24.0` | `MIT` | 传递 | — |
| `fontique` | `0.11.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `foreign-types` | `0.5.0` | `MIT/Apache-2.0` | 传递 | — |
| `foreign-types-macros` | `0.2.4` | `MIT/Apache-2.0` | 传递 | — |
| `foreign-types-shared` | `0.3.1` | `MIT/Apache-2.0` | 传递 | — |
| `form_urlencoded` | `1.2.2` | `MIT OR Apache-2.0` | 传递 | — |
| `futures` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-channel` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-core` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-executor` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-io` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-lite` | `2.6.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `futures-macro` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-sink` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-task` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `futures-util` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `gbm` | `0.18.0` | `MIT` | 传递 | — |
| `gbm-sys` | `0.4.0` | `MIT` | 传递 | — |
| `gethostname` | `1.1.0` | `Apache-2.0` | 传递 | — |
| `getrandom` | `0.3.4` | `MIT OR Apache-2.0` | 传递 | — |
| `getrandom` | `0.4.3` | `MIT OR Apache-2.0` | 传递 | — |
| `gif` | `0.14.2` | `MIT OR Apache-2.0` | 传递 | — |
| `gl_generator` | `0.14.0` | `Apache-2.0` | 传递 | — |
| `glob` | `0.3.4` | `MIT OR Apache-2.0` | 传递 | — |
| `glow` | `0.18.0` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `glutin` | `0.32.3` | `Apache-2.0` | 传递 | — |
| `glutin-winit` | `0.5.0` | `MIT` | 传递 | — |
| `glutin_egl_sys` | `0.7.1` | `Apache-2.0` | 传递 | — |
| `glutin_glx_sys` | `0.6.1` | `Apache-2.0` | 传递 | — |
| `glutin_wgl_sys` | `0.6.1` | `Apache-2.0` | 传递 | — |
| `gpu-allocator` | `0.28.0` | `MIT OR Apache-2.0` | 传递 | — |
| `grid` | `1.0.1` | `MIT` | 传递 | — |
| `half` | `2.7.1` | `MIT OR Apache-2.0` | 传递 | — |
| `harfrust` | `0.12.0` | `MIT` | 传递 | — |
| `hashbrown` | `0.14.5` | `MIT OR Apache-2.0` | 传递 | — |
| `hashbrown` | `0.15.5` | `MIT OR Apache-2.0` | 传递 | — |
| `hashbrown` | `0.16.1` | `MIT OR Apache-2.0` | 传递 | — |
| `hashbrown` | `0.17.1` | `MIT OR Apache-2.0` | 传递 | — |
| `heck` | `0.5.0` | `MIT OR Apache-2.0` | 传递 | — |
| `hermit-abi` | `0.3.9` | `MIT OR Apache-2.0` | 传递 | — |
| `hermit-abi` | `0.5.3` | `MIT OR Apache-2.0` | 传递 | — |
| `hex` | `0.4.3` | `MIT OR Apache-2.0` | 传递 | — |
| `hound` | `3.5.1` | `Apache-2.0` | 直接 | `yeban-render` |
| `htmlparser` | `0.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `hybrid-array` | `0.4.15` | `MIT OR Apache-2.0` | 传递 | — |
| `i-slint-backend-linuxkms` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-backend-selector` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-backend-testing` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 直接 | `yeban-ui-test-port` |
| `i-slint-backend-winit` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-common` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-compiler` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-core` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-core-macros` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-renderer-femtovg` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-renderer-skia` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `i-slint-renderer-software` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `iana-time-zone` | `0.1.65` | `MIT OR Apache-2.0` | 传递 | — |
| `iana-time-zone-haiku` | `0.1.2` | `MIT OR Apache-2.0` | 传递 | — |
| `icu_collections` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_decimal` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_decimal_data` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_locale_core` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_locale_fallback` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_locale_fallback_data` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_normalizer` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_normalizer_data` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_plurals` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_plurals_data` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_properties` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_properties_data` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_provider` | `2.3.1` | `Unicode-3.0` | 传递 | — |
| `icu_segmenter` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `icu_segmenter_data` | `2.3.0` | `Unicode-3.0` | 传递 | — |
| `idna` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `idna_adapter` | `1.2.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `image` | `0.25.10` | `MIT OR Apache-2.0` | 传递 | — |
| `image-webp` | `0.2.4` | `MIT OR Apache-2.0` | 传递 | — |
| `imagesize` | `0.15.0` | `MIT` | 传递 | — |
| `imgref` | `1.12.3` | `CC0-1.0 OR Apache-2.0` | 传递 | — |
| `indexmap` | `2.14.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `input` | `0.10.0` | `MIT` | 传递 | — |
| `input-sys` | `1.19.0` | `MIT` | 传递 | — |
| `io-lifetimes` | `1.0.11` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `itertools` | `0.13.0` | `MIT OR Apache-2.0` | 传递 | — |
| `itertools` | `0.15.0` | `MIT OR Apache-2.0` | 传递 | — |
| `itoa` | `1.0.18` | `MIT OR Apache-2.0` | 传递 | — |
| `jni` | `0.22.4` | `MIT OR Apache-2.0` | 传递 | — |
| `jni-macros` | `0.22.4` | `MIT OR Apache-2.0` | 传递 | — |
| `jni-sys` | `0.3.1` | `MIT OR Apache-2.0` | 传递 | — |
| `jni-sys` | `0.4.1` | `MIT OR Apache-2.0` | 传递 | — |
| `jni-sys-macros` | `0.4.1` | `MIT OR Apache-2.0` | 传递 | — |
| `jobserver` | `0.1.35` | `MIT OR Apache-2.0` | 传递 | — |
| `js-sys` | `0.3.106` | `MIT OR Apache-2.0` | 传递 | — |
| `keyboard-types` | `0.7.0` | `MIT OR Apache-2.0` | 传递 | — |
| `khronos_api` | `3.1.0` | `Apache-2.0` | 传递 | — |
| `kurbo` | `0.13.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `lazy_static` | `1.5.1` | `MIT OR Apache-2.0` | 传递 | — |
| `libc` | `0.2.190` | `MIT OR Apache-2.0` | 传递 | — |
| `libloading` | `0.8.9` | `ISC` | 传递 | — |
| `libm` | `0.2.16` | `MIT` | 直接 | `yeban-mcp`, `yeban-render`, `yeban-theory` |
| `libredox` | `0.1.25` | `MIT` | 传递 | — |
| `libseat` | `0.2.4` | `MIT` | 传递 | — |
| `libseat-sys` | `0.2.0` | `MIT` | 传递 | — |
| `libudev-sys` | `0.1.4` | `MIT` | 传递 | — |
| `linebender_resource_handle` | `0.1.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `linux-raw-sys` | `0.12.1` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `linux-raw-sys` | `0.4.15` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `linux-raw-sys` | `0.9.4` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `litemap` | `0.8.3` | `Unicode-3.0` | 传递 | — |
| `litrs` | `1.0.0` | `MIT OR Apache-2.0` | 传递 | — |
| `lock_api` | `0.4.14` | `MIT OR Apache-2.0` | 传递 | — |
| `log` | `0.4.34` | `MIT OR Apache-2.0` | 传递 | — |
| `lyon_algorithms` | `1.0.21` | `MIT OR Apache-2.0` | 传递 | — |
| `lyon_extra` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `lyon_geom` | `1.0.19` | `MIT OR Apache-2.0` | 传递 | — |
| `lyon_path` | `1.0.19` | `MIT OR Apache-2.0` | 传递 | — |
| `mach2` | `0.6.0` | `BSD-2-Clause OR MIT OR Apache-2.0` | 传递 | — |
| `memchr` | `2.8.3` | `Unlicense OR MIT` | 传递 | — |
| `memmap2` | `0.9.11` | `MIT OR Apache-2.0` | 传递 | — |
| `memoffset` | `0.9.1` | `MIT` | 传递 | — |
| `midly` | `0.5.3` | `Unlicense` | 直接 | `yeban-midi`, `yeban-render` |
| `minimal-lexical` | `0.2.1` | `MIT/Apache-2.0` | 传递 | — |
| `miniz_oxide` | `0.8.9` | `MIT OR Zlib OR Apache-2.0` | 传递 | — |
| `miniz_oxide` | `0.9.1` | `MIT OR Zlib OR Apache-2.0` | 传递 | — |
| `moxcms` | `0.8.1` | `BSD-3-Clause OR Apache-2.0` | 传递 | — |
| `muda` | `0.19.3` | `Apache-2.0 OR MIT` | 传递 | — |
| `naga` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `naga-types` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `natord` | `1.0.9` | `MIT` | 传递 | — |
| `ndk` | `0.9.0` | `MIT OR Apache-2.0` | 传递 | — |
| `ndk-context` | `0.1.1` | `MIT OR Apache-2.0` | 传递 | — |
| `ndk-sys` | `0.6.0+11769913` | `MIT OR Apache-2.0` | 传递 | — |
| `nix` | `0.31.3` | `MIT` | 传递 | — |
| `nom` | `7.1.3` | `MIT` | 传递 | — |
| `nom` | `8.0.0` | `MIT` | 传递 | — |
| `num-complex` | `0.4.6` | `MIT OR Apache-2.0` | 传递 | — |
| `num-derive` | `0.4.2` | `MIT OR Apache-2.0` | 传递 | — |
| `num-integer` | `0.1.47` | `MIT OR Apache-2.0` | 传递 | — |
| `num-traits` | `0.2.19` | `MIT OR Apache-2.0` | 传递 | — |
| `num_enum` | `0.7.6` | `BSD-3-Clause OR MIT OR Apache-2.0` | 传递 | — |
| `num_enum_derive` | `0.7.6` | `BSD-3-Clause OR MIT OR Apache-2.0` | 传递 | — |
| `objc-sys` | `0.3.5` | `MIT` | 传递 | — |
| `objc2` | `0.5.2` | `MIT` | 传递 | — |
| `objc2` | `0.6.4` | `MIT` | 传递 | — |
| `objc2-app-kit` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-app-kit` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-audio-toolbox` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-avf-audio` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-cloud-kit` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-contacts` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-core-audio` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-core-audio-types` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-core-data` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-core-foundation` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-core-graphics` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-core-image` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-core-location` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-core-text` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-encode` | `4.1.0` | `MIT` | 传递 | — |
| `objc2-foundation` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-foundation` | `0.3.2` | `MIT` | 传递 | — |
| `objc2-io-surface` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-link-presentation` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-metal` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-metal` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-quartz-core` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-quartz-core` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-symbols` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-ui-kit` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-ui-kit` | `0.3.2` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `objc2-uniform-type-identifiers` | `0.2.2` | `MIT` | 传递 | — |
| `objc2-user-notifications` | `0.2.2` | `MIT` | 传递 | — |
| `once_cell` | `1.21.4` | `MIT OR Apache-2.0` | 传递 | — |
| `orbclient` | `0.3.55` | `MIT` | 传递 | — |
| `ordered-float` | `5.5.0` | `MIT` | 传递 | — |
| `ordered-stream` | `0.2.0` | `MIT OR Apache-2.0` | 传递 | — |
| `os_pipe` | `1.2.3` | `MIT` | 传递 | — |
| `owned_ttf_parser` | `0.25.1` | `Apache-2.0` | 传递 | — |
| `parking` | `2.2.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `parking_lot` | `0.12.5` | `MIT OR Apache-2.0` | 传递 | — |
| `parking_lot_core` | `0.9.12` | `MIT OR Apache-2.0` | 传递 | — |
| `parlance` | `0.1.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `parley` | `0.11.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `parley_data` | `0.11.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `percent-encoding` | `2.3.2` | `MIT OR Apache-2.0` | 传递 | — |
| `petgraph` | `0.8.3` | `MIT OR Apache-2.0` | 传递 | — |
| `phf` | `0.13.1` | `MIT` | 传递 | — |
| `phf_generator` | `0.13.1` | `MIT` | 传递 | — |
| `phf_macros` | `0.13.1` | `MIT` | 传递 | — |
| `phf_shared` | `0.13.1` | `MIT` | 传递 | — |
| `pico-args` | `0.5.0` | `MIT` | 传递 | — |
| `pin-project` | `1.1.13` | `Apache-2.0 OR MIT` | 传递 | — |
| `pin-project-internal` | `1.1.13` | `Apache-2.0 OR MIT` | 传递 | — |
| `pin-project-lite` | `0.2.17` | `Apache-2.0 OR MIT` | 传递 | — |
| `pin-utils` | `0.1.1` | `MIT OR Apache-2.0` | 传递 | — |
| `pin-weak` | `1.1.0` | `MIT` | 传递 | — |
| `piper` | `0.2.5` | `MIT OR Apache-2.0` | 传递 | — |
| `pkg-config` | `0.3.34` | `MIT OR Apache-2.0` | 传递 | — |
| `plain` | `0.2.3` | `MIT/Apache-2.0` | 传递 | — |
| `png` | `0.18.1` | `MIT OR Apache-2.0` | 传递 | — |
| `polling` | `3.11.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `polycool` | `0.4.0` | `MIT OR Apache-2.0` | 传递 | — |
| `portable-atomic` | `1.15.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `portable-atomic-util` | `0.2.8` | `Apache-2.0 OR MIT` | 传递 | — |
| `potential_utf` | `0.1.6` | `Unicode-3.0` | 传递 | — |
| `ppv-lite86` | `0.2.21` | `MIT OR Apache-2.0` | 传递 | — |
| `presser` | `0.3.1` | `MIT OR Apache-2.0` | 传递 | — |
| `prettyplease` | `0.2.37` | `MIT OR Apache-2.0` | 传递 | — |
| `proc-macro-crate` | `3.5.0` | `MIT OR Apache-2.0` | 传递 | — |
| `proc-macro2` | `1.0.107` | `MIT OR Apache-2.0` | 传递 | — |
| `profiling` | `1.0.18` | `MIT OR Apache-2.0` | 传递 | — |
| `proptest` | `1.11.0` | `MIT OR Apache-2.0` | 直接 | `yeban-decode`, `yeban-model`, `yeban-render`, `yeban-theory` |
| `pulldown-cmark` | `0.13.4` | `MIT` | 传递 | — |
| `pxfm` | `0.1.30` | `BSD-3-Clause OR Apache-2.0` | 传递 | — |
| `quick-error` | `2.0.1` | `MIT/Apache-2.0` | 传递 | — |
| `quick-xml` | `0.41.0` | `MIT` | 传递 | — |
| `quote` | `1.0.47` | `MIT OR Apache-2.0` | 传递 | — |
| `r-efi` | `5.3.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 传递 | — |
| `r-efi` | `6.0.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 传递 | — |
| `rand` | `0.10.3` | `MIT OR Apache-2.0` | 传递 | — |
| `rand` | `0.9.5` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_chacha` | `0.9.0` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_core` | `0.10.1` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_core` | `0.9.5` | `MIT OR Apache-2.0` | 传递 | — |
| `rand_xorshift` | `0.4.0` | `MIT OR Apache-2.0` | 传递 | — |
| `range-alloc` | `0.1.5` | `MIT OR Apache-2.0` | 传递 | — |
| `raw-window-handle` | `0.6.2` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `raw-window-metal` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `rayon` | `1.12.0` | `MIT OR Apache-2.0` | 直接 | `yeban-render` |
| `rayon-core` | `1.13.0` | `MIT OR Apache-2.0` | 传递 | — |
| `read-fonts` | `0.41.0` | `MIT OR Apache-2.0` | 传递 | — |
| `redox_syscall` | `0.4.1` | `MIT` | 传递 | — |
| `redox_syscall` | `0.5.18` | `MIT` | 传递 | — |
| `redox_syscall` | `0.9.4` | `MIT` | 传递 | — |
| `regex` | `1.13.1` | `MIT OR Apache-2.0` | 传递 | — |
| `regex-automata` | `0.4.18` | `MIT OR Apache-2.0` | 传递 | — |
| `regex-lite` | `0.1.9` | `MIT OR Apache-2.0` | 传递 | — |
| `regex-syntax` | `0.8.11` | `MIT OR Apache-2.0` | 传递 | — |
| `renderdoc-sys` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `resvg` | `0.48.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `rgb` | `0.8.53` | `MIT` | 传递 | — |
| `rowan` | `0.17.0` | `MIT OR Apache-2.0` | 传递 | — |
| `roxmltree` | `0.21.1` | `MIT OR Apache-2.0` | 传递 | — |
| `rspolib` | `0.1.2` | `MIT` | 传递 | — |
| `rtrb` | `0.4.0` | `MIT OR Apache-2.0` | 直接 | `yeban-engine` |
| `rubato` | `5.0.1` | `MIT OR Apache-2.0` | 直接 | `yeban-decode` |
| `rustc-hash` | `1.1.0` | `Apache-2.0/MIT` | 传递 | — |
| `rustc-hash` | `2.1.3` | `Apache-2.0 OR MIT` | 传递 | — |
| `rustc_version` | `0.4.1` | `MIT OR Apache-2.0` | 传递 | — |
| `rustix` | `0.38.44` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `rustix` | `1.1.5` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `rustversion` | `1.0.23` | `MIT OR Apache-2.0` | 传递 | — |
| `same-file` | `1.0.6` | `Unlicense/MIT` | 传递 | — |
| `scoped-tls` | `1.0.1` | `MIT/Apache-2.0` | 传递 | — |
| `scoped-tls-hkt` | `0.1.5` | `MIT/Apache-2.0` | 传递 | — |
| `scopeguard` | `1.2.0` | `MIT OR Apache-2.0` | 传递 | — |
| `sctk-adwaita` | `0.10.1` | `MIT` | 传递 | — |
| `semver` | `1.0.28` | `MIT OR Apache-2.0` | 传递 | — |
| `serde` | `1.0.229` | `MIT OR Apache-2.0` | 直接 | `yeban-mcp`, `yeban-model`, `yeban-ui-mcp`, `yeban-ui-test-port` |
| `serde_core` | `1.0.229` | `MIT OR Apache-2.0` | 传递 | — |
| `serde_derive` | `1.0.229` | `MIT OR Apache-2.0` | 传递 | — |
| `serde_json` | `1.0.151` | `MIT OR Apache-2.0` | 直接 | `yeban-mcp`, `yeban-model`, `yeban-ui-mcp`, `yeban-ui-test-port` |
| `serde_repr` | `0.1.21` | `MIT OR Apache-2.0` | 传递 | — |
| `serde_spanned` | `1.1.1` | `MIT OR Apache-2.0` | 传递 | — |
| `sha2` | `0.11.0` | `MIT OR Apache-2.0` | 直接 | `yeban-diagnostics`, `yeban-model`, `yeban-render` |
| `shlex` | `1.3.0` | `MIT OR Apache-2.0` | 传递 | — |
| `shlex` | `2.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `signal-hook-registry` | `1.4.8` | `MIT OR Apache-2.0` | 传递 | — |
| `signalsmith-stretch` | `0.1.3` | `MIT` | 直接 | `yeban-dsp` |
| `simd-adler32` | `0.3.10` | `MIT` | 传递 | — |
| `simd_cesu8` | `1.2.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `simdutf8` | `0.1.5` | `MIT OR Apache-2.0` | 传递 | — |
| `simplecss` | `0.2.2` | `Apache-2.0 OR MIT` | 传递 | — |
| `siphasher` | `1.0.4` | `MIT OR Apache-2.0` | 传递 | — |
| `skia-bindings` | `0.153.3` | `MIT` | 传递 | — |
| `skia-safe` | `0.153.3` | `MIT` | 传递 | — |
| `skrifa` | `0.44.0` | `MIT OR Apache-2.0` | 传递 | — |
| `slab` | `0.4.12` | `MIT` | 传递 | — |
| `slint` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 直接 | `yeban-app`, `yeban-ui-test-port` |
| `slint-build` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 直接 | `yeban-app`, `yeban-ui-test-port` |
| `slint-macros` | `1.18.1` | `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0` | 传递 | — |
| `slotmap` | `1.1.1` | `Zlib` | 传递 | — |
| `smallvec` | `1.16.2` | `MIT OR Apache-2.0` | 传递 | — |
| `smithay-client-toolkit` | `0.19.2` | `MIT` | 传递 | — |
| `smol_str` | `0.2.2` | `MIT OR Apache-2.0` | 传递 | — |
| `smol_str` | `0.3.6` | `MIT OR Apache-2.0` | 传递 | — |
| `snafu` | `0.8.9` | `MIT OR Apache-2.0` | 传递 | — |
| `snafu-derive` | `0.8.9` | `MIT OR Apache-2.0` | 传递 | — |
| `softbuffer` | `0.4.8` | `MIT OR Apache-2.0` | 传递 | — |
| `spin_on` | `0.1.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `spirv` | `0.4.0+sdk-1.4.341.0` | `Apache-2.0` | 传递 | — |
| `stable_deref_trait` | `1.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `static_assertions` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `strict-num` | `0.1.1` | `MIT` | 传递 | — |
| `strum` | `0.28.0` | `MIT` | 传递 | — |
| `strum_macros` | `0.28.0` | `MIT` | 传递 | — |
| `svgtypes` | `0.16.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `swash` | `0.2.10` | `Apache-2.0 OR MIT` | 传递 | — |
| `symphonia` | `0.6.1` | `MPL-2.0` | 直接 | `yeban-decode` |
| `symphonia-bundle-flac` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-codec-adpcm` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-codec-pcm` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-codec-vorbis` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-common` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-core` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-format-ogg` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-format-riff` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `symphonia-metadata` | `0.6.1` | `MPL-2.0` | 传递 | — |
| `syn` | `2.0.119` | `MIT OR Apache-2.0` | 传递 | — |
| `syn` | `3.0.6` | `MIT OR Apache-2.0` | 传递 | — |
| `synstructure` | `0.14.0` | `MIT` | 传递 | — |
| `sys-locale` | `0.3.2` | `MIT OR Apache-2.0` | 传递 | — |
| `taffy` | `0.10.1` | `MIT` | 传递 | — |
| `tar` | `0.4.46` | `MIT OR Apache-2.0` | 传递 | — |
| `tempfile` | `3.27.0` | `MIT OR Apache-2.0` | 直接 | `yeban-render` |
| `text-size` | `1.1.1` | `MIT OR Apache-2.0` | 传递 | — |
| `thiserror` | `1.0.69` | `MIT OR Apache-2.0` | 直接 | `yeban-decode`, `yeban-engine`, `yeban-mcp`, `yeban-model`, `yeban-sfz`, `yeban-theory`, `yeban-ui-mcp` |
| `thiserror` | `2.0.21` | `MIT OR Apache-2.0` | 直接 | `yeban-decode`, `yeban-engine`, `yeban-mcp`, `yeban-model`, `yeban-sfz`, `yeban-theory`, `yeban-ui-mcp` |
| `thiserror-impl` | `1.0.69` | `MIT OR Apache-2.0` | 传递 | — |
| `thiserror-impl` | `2.0.21` | `MIT OR Apache-2.0` | 传递 | — |
| `tiny-skia` | `0.11.4` | `BSD-3-Clause` | 传递 | — |
| `tiny-skia` | `0.12.0` | `BSD-3-Clause` | 传递 | — |
| `tiny-skia-path` | `0.11.4` | `BSD-3-Clause` | 传递 | — |
| `tiny-skia-path` | `0.12.0` | `BSD-3-Clause` | 传递 | — |
| `tiny-xlib` | `0.2.5` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `tinystr` | `0.8.4` | `Unicode-3.0` | 传递 | — |
| `tinyvec` | `1.13.3` | `Zlib OR Apache-2.0 OR MIT` | 传递 | — |
| `tokio` | `1.53.2` | `MIT` | 传递 | — |
| `toml` | `1.1.6+spec-1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `toml_datetime` | `1.1.1+spec-1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `toml_edit` | `0.25.15+spec-1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `toml_parser` | `1.1.3+spec-1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `toml_writer` | `1.1.2+spec-1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `tracing` | `0.1.44` | `MIT` | 传递 | — |
| `tracing-attributes` | `0.1.31` | `MIT` | 传递 | — |
| `tracing-core` | `0.1.36` | `MIT` | 传递 | — |
| `tree_magic_mini` | `3.2.2` | `MIT` | 传递 | — |
| `ttf-parser` | `0.25.1` | `MIT OR Apache-2.0` | 传递 | — |
| `typed-index-collections` | `3.5.0` | `MIT OR Apache-2.0` | 传递 | — |
| `typed-path` | `0.12.3` | `MIT OR Apache-2.0` | 传递 | — |
| `typenum` | `1.20.1` | `MIT OR Apache-2.0` | 传递 | — |
| `udev` | `0.9.3` | `MIT` | 传递 | — |
| `uds_windows` | `1.2.1` | `MIT` | 传递 | — |
| `ulid` | `3.0.0` | `MIT` | 直接 | `yeban-model` |
| `unarray` | `0.1.4` | `MIT OR Apache-2.0` | 传递 | — |
| `unicase` | `2.9.0` | `MIT OR Apache-2.0` | 传递 | — |
| `unicode-bidi` | `0.3.18` | `MIT OR Apache-2.0` | 传递 | — |
| `unicode-ident` | `1.0.26` | `(MIT OR Apache-2.0) AND Unicode-3.0` | 传递 | — |
| `unicode-linebreak` | `0.1.5` | `Apache-2.0` | 传递 | — |
| `unicode-script` | `0.5.8` | `MIT OR Apache-2.0` | 传递 | — |
| `unicode-segmentation` | `1.13.3` | `MIT OR Apache-2.0` | 传递 | — |
| `unicode-vo` | `0.1.0` | `MIT/Apache-2.0` | 传递 | — |
| `unicode-width` | `0.2.2` | `MIT OR Apache-2.0` | 传递 | — |
| `unicode-xid` | `0.2.6` | `MIT OR Apache-2.0` | 传递 | — |
| `unty` | `0.0.4` | `MIT OR Apache-2.0` | 传递 | — |
| `url` | `2.5.8` | `MIT OR Apache-2.0` | 传递 | — |
| `usvg` | `0.48.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `utf8_iter` | `1.0.4` | `Apache-2.0 OR MIT` | 传递 | — |
| `uuid` | `1.27.0` | `Apache-2.0 OR MIT` | 传递 | — |
| `version_check` | `0.9.5` | `MIT/Apache-2.0` | 传递 | — |
| `visibility` | `0.1.1` | `Zlib OR MIT OR Apache-2.0` | 传递 | — |
| `vtable` | `0.5.0` | `MIT OR Apache-2.0` | 传递 | — |
| `vtable-macro` | `0.5.0` | `MIT OR Apache-2.0` | 传递 | — |
| `walkdir` | `2.5.0` | `Unlicense/MIT` | 传递 | — |
| `wasip2` | `1.0.1+wasi-0.2.4` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `wasm-bindgen` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-futures` | `0.4.79` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-macro` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-macro-support` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wasm-bindgen-shared` | `0.2.129` | `MIT OR Apache-2.0` | 传递 | — |
| `wayland-backend` | `0.3.17` | `MIT` | 传递 | — |
| `wayland-client` | `0.31.15` | `MIT` | 传递 | — |
| `wayland-csd-frame` | `0.3.0` | `MIT` | 传递 | — |
| `wayland-cursor` | `0.31.14` | `MIT` | 传递 | — |
| `wayland-protocols` | `0.32.13` | `MIT` | 传递 | — |
| `wayland-protocols-plasma` | `0.3.12` | `MIT` | 传递 | — |
| `wayland-protocols-wlr` | `0.3.12` | `MIT` | 传递 | — |
| `wayland-scanner` | `0.31.11` | `MIT` | 传递 | — |
| `wayland-sys` | `0.31.11` | `MIT` | 传递 | — |
| `web-sys` | `0.3.106` | `MIT OR Apache-2.0` | 传递 | — |
| `web-time` | `1.1.0` | `MIT OR Apache-2.0` | 传递 | — |
| `webbrowser` | `1.2.4` | `MIT OR Apache-2.0` | 传递 | — |
| `weezl` | `0.1.12` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu-core` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu-core-deps-apple` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu-core-deps-windows-linux-android` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu-hal` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu-naga-bridge` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `wgpu-types` | `30.0.1` | `MIT OR Apache-2.0` | 传递 | — |
| `winapi-util` | `0.1.11` | `Unlicense OR MIT` | 传递 | — |
| `windowfunctions` | `0.1.1` | `MIT` | 传递 | — |
| `windows` | `0.62.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-collections` | `0.3.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-core` | `0.62.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-future` | `0.3.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-implement` | `0.60.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-interface` | `0.59.3` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-link` | `0.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-numerics` | `0.3.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-result` | `0.4.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-strings` | `0.5.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-sys` | `0.48.0` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-sys` | `0.52.0` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-sys` | `0.59.0` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-sys` | `0.60.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-sys` | `0.61.2` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-targets` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-targets` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-targets` | `0.53.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows-threading` | `0.2.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_aarch64_gnullvm` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_aarch64_gnullvm` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_aarch64_gnullvm` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_aarch64_msvc` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_aarch64_msvc` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_aarch64_msvc` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_gnu` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_gnu` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_gnu` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_gnullvm` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_gnullvm` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_msvc` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_msvc` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_i686_msvc` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_gnu` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_gnu` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_gnu` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_gnullvm` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_gnullvm` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_gnullvm` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_msvc` | `0.48.5` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_msvc` | `0.52.6` | `MIT OR Apache-2.0` | 传递 | — |
| `windows_x86_64_msvc` | `0.53.1` | `MIT OR Apache-2.0` | 传递 | — |
| `winit` | `0.30.13` | `Apache-2.0` | 传递 | — |
| `winnow` | `1.0.4` | `MIT` | 传递 | — |
| `wit-bindgen` | `0.46.0` | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 传递 | — |
| `wl-clipboard-rs` | `0.9.4` | `MIT OR Apache-2.0` | 传递 | — |
| `write-fonts` | `0.50.0` | `MIT OR Apache-2.0` | 传递 | — |
| `writeable` | `0.6.4` | `Unicode-3.0` | 传递 | — |
| `x11-dl` | `2.21.0` | `MIT` | 传递 | — |
| `x11rb` | `0.13.2` | `MIT OR Apache-2.0` | 传递 | — |
| `x11rb-protocol` | `0.13.2` | `MIT OR Apache-2.0` | 传递 | — |
| `xattr` | `1.6.1` | `MIT OR Apache-2.0` | 传递 | — |
| `xcursor` | `0.3.11` | `MIT` | 传递 | — |
| `xkbcommon` | `0.9.0` | `MIT` | 传递 | — |
| `xkbcommon-dl` | `0.4.2` | `MIT` | 传递 | — |
| `xkeysym` | `0.2.1` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `xml-rs` | `0.8.29` | `MIT` | 传递 | — |
| `xmlwriter` | `0.1.0` | `MIT` | 传递 | — |
| `yazi` | `0.2.1` | `Apache-2.0 OR MIT` | 传递 | — |
| `yeslogic-fontconfig-sys` | `6.0.1` | `MIT` | 传递 | — |
| `yoke` | `0.8.3` | `Unicode-3.0` | 传递 | — |
| `yoke-derive` | `0.8.4` | `Unicode-3.0` | 传递 | — |
| `zbus` | `5.19.0` | `MIT` | 传递 | — |
| `zbus-lockstep` | `0.5.2` | `MIT` | 传递 | — |
| `zbus-lockstep-macros` | `0.5.2` | `MIT` | 传递 | — |
| `zbus_macros` | `5.19.0` | `MIT` | 传递 | — |
| `zbus_names` | `4.3.4` | `MIT` | 传递 | — |
| `zbus_xml` | `5.2.1` | `MIT` | 传递 | — |
| `zcheapstr` | `1.1.0` | `MIT` | 传递 | — |
| `zeno` | `0.3.3` | `Apache-2.0 OR MIT` | 传递 | — |
| `zerocopy` | `0.8.59` | `BSD-2-Clause OR Apache-2.0 OR MIT` | 传递 | — |
| `zerocopy-derive` | `0.8.59` | `BSD-2-Clause OR Apache-2.0 OR MIT` | 传递 | — |
| `zerofrom` | `0.1.8` | `Unicode-3.0` | 传递 | — |
| `zerofrom-derive` | `0.1.8` | `Unicode-3.0` | 传递 | — |
| `zerotrie` | `0.2.5` | `Unicode-3.0` | 传递 | — |
| `zerovec` | `0.11.8` | `Unicode-3.0` | 传递 | — |
| `zerovec-derive` | `0.11.6` | `Unicode-3.0` | 传递 | — |
| `zip` | `8.6.0` | `MIT` | 直接 | `yeban-diagnostics` |
| `zlib-rs` | `0.6.8` | `Zlib` | 传递 | — |
| `zmij` | `1.0.23` | `MIT` | 传递 | — |
| `zune-core` | `0.5.3` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `zune-jpeg` | `0.5.15` | `MIT OR Apache-2.0 OR Zlib` | 传递 | — |
| `zvariant` | `5.15.0` | `MIT` | 传递 | — |
| `zvariant_derive` | `5.15.0` | `MIT` | 传递 | — |
| `zvariant_utils` | `4.2.0` | `MIT` | 传递 | — |

## 源码级移植（不由 cargo 依赖图覆盖）

从**历史项目源码**改写移植进来的代码不在 `cargo deny` 视野内，归属与许可全文必须由 `THIRD_PARTY_LICENSES.md` 承载。当前登记：

- `synth-core`（MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors）—
  `crates/yeban-dsp` 的部分模块；逐文件裁决见 `docs/ledger/dsp-core-provenance.md`。
