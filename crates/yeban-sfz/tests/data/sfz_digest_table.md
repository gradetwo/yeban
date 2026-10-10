# `yeban-sfz` 归约摘要表（字面契约）

本表是 **R70②／R78④** 要求的「第二方向」：判据里的 `DIGEST_*` 字面常量**必须**同时出现在
本文件里，而本文件由 `include_str!` 在**编译期**读入 ⇒ 代码与文档任一方向漂移都会让判据变红。

| 摘要常量 | 用途 | 值 |
| :-- | :-- | :-- |
| `DIGEST_INCLUDE_RESOLUTION` | `include_resolution_is_deterministic_and_its_digest_is_pinned`：`#include "parts/*.sfz"` 展开后的 **路径序列 + 归约样本序列** 的规范摘要 | `025e07f2edb4e6609dd2e6aaaa8936fab5fd79f0a88e6e9997b529e454a54b14` |
