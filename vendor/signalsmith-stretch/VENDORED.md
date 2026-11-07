# Vendored: signalsmith-stretch 0.1.3

- 上游：`signalsmith-stretch` 0.1.3（crates.io，https://github.com/colinmarc/signalsmith-stretch-rs），MIT 许可，Copyright 2024 Colin Marc。
- 入库时间：2026-11-07，随 vocaledit 拉伸抖动修复（`line/vocaledit-jitter-fix`）。
- 入库原因：上游默认构造以 `std::random_device` 播种内部 RNG，拉伸倍数超过 2.0 的段逐轮输出不同（见 `~/workspace/vocaledit-phase2/REPORT-jitter-diagnosis.md`）；上游没有种子接口可调，须 vendor 后加播种构造。
- 许可：`LICENSE.md` 原样保留（MIT）；本目录其余上游文件未改动。

## 相对上游 0.1.3 的改动（仅三处）

1. `src/wrapper.h`：新增三个带种子创建函数声明（`signalsmith_stretch_create_seeded` / `signalsmith_stretch_create_preset_default_seeded` / `signalsmith_stretch_create_preset_cheaper_seeded`）。
2. `src/wrapper.cpp`：`struct signalsmith_stretch` 增加带种子构造（调 `SignalsmithStretch(long seed)`）；新增三个带种子创建函数实现。原有无种子创建函数行为不变。
3. `src/lib.rs`：`Stretch` 增加 `new_seeded` / `preset_default_seeded` / `preset_cheaper_seeded`。

另删 `Cargo.lock`（path 依赖不读它；工作区以根 `Cargo.lock` 为准）。

## 保证边界

固定种子只保证**同一构建、同一平台、同一调用序列**下的逐位相同，不宣称跨工具链逐位相同（FFT 后端、浮点求值顺序仍可变）。上游若发布带种子接口的新版，可删 vendor、改回 registry 依赖并重验。
