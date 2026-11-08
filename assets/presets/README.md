# 工厂预制（系统预制，只读）

设计稿 `~/workspace/yeban-preset-design.md` §3 的首批 6 件。格式见
`crates/yeban-model/src/preset.rs`（格式 v1，单份上限 64 KiB）。

- 标识：`sys:<乐器 slug>:<预制名>`；每乐器一个默认预制，绑定乐器时
  `track.preset.autoLoad` 缺省 `true` 即自动应用。
- 全部是乐器调优（`scope: instrument`）：只动 instrument、strip、sends
  三类；不动混音、不动路由结构。
- **数值全是草案值，待耳验。** 耳验门是 Wang 的耳朵，数字只诊断。
  散文未给具体值的地方（如吉他高通频率、长笛气息频段量、混响/卷积对象
  的内部参数）暂取缺省或同类值，各文件 `draftNote` 里写明，耳验时定。
- 宏的 `mappings` 目前为空：宏名与位置随预制走，绑定到通道条/发送参数
  的映射待引擎侧宏目标落定后补（不编造目标）。
