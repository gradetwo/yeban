# D3＋D4 接线 — PROGRESS

基线：origin/main = 85e2953e8（2026-11-19 开工 ls-remote 现读，与简报一致）。
分支：line/drum-shaping-wiring；工作树 ~/workspace/wt-drum-shaping。
用盒：yanshi-dev（hostname instabox-yanshi-dev 已核）；/data 起始余量 11 GB（df 实测）。
标记：~/workspace/tools/.box-task/yanshi-dev 已建，每步 touch。

## 设计裁决（开工定，执行中不改）

- 拼写（单拼写、带前缀，照 limiter/clipper 惯例；裸 `attack` 已被 StripField 占为压缩起振别名）：
  - 瞬态：`transient_attack`（−1..1，缺省 0）、`transient_sustain`（−1..1，缺省 0）
  - 碾碎：`crush_bits`（4..16，缺省 16）、`crush_down`（1..64，缺省 1）、`crush_aa`（0..1，缺省 0）
  - 全仓 grep 已核：五名与既有全部 SPELLINGS 无撞名。
- 链序（InsertRuntime::process / process_with_key 同序）：
  sample_delay → strip → **transient** → tape → **crusher** → clipper → reverb → convolution → delay → limiter。
  理由：瞬态包络跟随器看通道条塑形后、磁带柔化前的信号；碾碎是介质退化级，
  与其它染色级同段（磁带后、削波前），削波/限制仍兜底碾碎后的峰值；
  混响/延迟吃的是碾碎后的干声（尾巴是「碾碎鼓的混响」，不是「混响被碾碎」）。
- 建级门：瞬态按 limiter/delay 惯例——投影为 Some 才建；参数恒等（0,0 逐位恒等，dsp 判据已钉）
  且未强制时不建（状态不推进与恒等同解）。碾碎缺省（16,1,0）**不是**逐位恒等
  （16 bit 量化步长 3.05e-5 真实改低位）⇒ 投影为 Some 即建，不设恒等门。
- 实时引擎（yeban-engine/src/insert.rs）不接：与 limiter/clipper（C-1）同一先例——
  最近两族均为离线/渲染侧接线；逐件链本身只有渲染侧存在。本线范围 = 渲染侧全链
  （轨道链＋逐件链）＋自动化落点＋MCP 公告面。
- 参数结构补 `DEFAULT`/`new()`/`sanitised()`（照 ClipperParams）：CrushParams、
  TransientParams 的 sanitised 与 process 入口钳制逐字同式（器件钳制是唯一一份，
  sanitise 只提前算给恒等判定/投影用）。
- BitCrusher 补 `process_in_place`（照 TransientShaper 既有同名方法逐式镜像；
  逐样本先读后写，别名安全），运行态因此与瞬态同形调用。

## 落点清单（计划）

1. yeban-dsp/src/shaping.rs：两参数结构 DEFAULT/new/sanitised；BitCrusher::process_in_place。
2. yeban-dsp/src/device_param.rs：TransientField（2 字段）、CrusherField（3 字段）；
   DeviceParamField/DeviceParamFamily 各 +2 变体；模块头表更新。
3. yeban-render/src/inserts.rs：InsertChain +transient/crusher 槽位与 force 位；
   transient_of/crusher_of 投影；*_source_slot/*_source_params；ForcedInsertInstances +2 集；
   InsertRuntime 实例化/reset/apply/process/process_with_key；re-export +2。
4. yeban-render/src/automation.rs：APPLIED_DEVICE_PARAM_NAMES +5（110→115 拼写、64→69 字段）；
   static_insert_field_value、applicable 家族臂、forced_insert_instances 各 +2 臂；
   INSERT_FIELD_CASES +5；一致性判据计数更新。
5. yeban-mcp/src/effect_params.rs：TRANSIENT、CRUSHER 两组公告（EFFECT_GROUPS 12→14）；
   yeban-app 经 #[path] 共用同一文件，自动同步。
6. yeban-theory/src/drum_preset.rs：DrumTransientPreset / DrumCrusherPreset；
   fx 与 lane chain 各 +2 可选段（parse 共用、空链拒绝枚举同步）；
   DrumFxFamilies +transient/crusher；drum_fx_device_specs / drum_lane_device_specs 产新器件
   （名 "Transient Shaper" / "Bitcrusher"，family "transient" / "crusher"）。
7. funk.json：fx.transient {attack 0.45, sustain 0.15}。
   lofi.json：fx.crusher {bits 10, down 2, aa 0.35}；source.tuning 追加一条接线说明
   （原文不改；D5 噪声、P7 wow/flutter 仍缺，注明）。
8. 生成侧在场扫描同步：yeban-app/src/bridge.rs drum_fx_families_present +2 族；
   yeban-mcp/src/domain/mod.rs 同处 +2 族。
9. app bridge 判据 drum_gen_ops_factory_styles_all_carry_the_full_fx_layer：
   期望器件数 +transient/crusher 项，名字识别断言 +2 族。

## 验证计划

- dsp：shaping 既有测试 + 新增 crusher in-place 对照判据；device_param 新表判据。
- render：insert_chain / automation 改动相关套件；新判据（投影、运行渲染差分、逐件链、自动化落点）须自证能红。
- theory：drum_preset 套件（解析/配方/出厂风格全量）。
- mcp：--lib 全套 + effect_contract + 普查判据全套（assertion_bounds_audit、param_bounds_guard、r77、license_inventory --check 等波及项）。
- app：改动相关（drum 生成判据、device_clip_creation）；分批跑。
- clippy --all-targets -D warnings（所改 crate）；fmt。
- 差分留证：funk 开/关 transient、lofi 开/关 crusher 的渲染差（逐样本差计数 + 峰值/RMS dB），
  判据内实测、回执报数。

## 进度

- [x] 探索与设计（2026-11-19 上午）
- [x] dsp 层（shaping 参数结构 + device_param 两族；VM cargo check 过；dsp 测试 VM 先行：lib 568/0、crusher_in_place 3/0、non_finite 11/0；transient_in_place 一行被 head 截断未见，盒上全套复核）
- [x] render 层（inserts + automation；VM check 过；修一处重复导入 E0252：两新字段只走 re-export 块）
- [x] mcp 公告面（effect_params 两组 + props_176/trap_175 计数钉 118→123、69→74；VM check 过）
- [x] theory 预置面 + 两 JSON（DrumTransientPreset/DrumCrusherPreset、fx/lane 双落点、配方两函数、funk/lofi 定值与 lofi 出处追加）
- [x] 生成侧扫描 + app 判据（bridge 与 mcp domain 在场扫描 +2 族；bridge 判据期望与识别断言同步）
- [x] 新判据：render 集成 shaping_chain_audio（4 条，funk/lofi 出厂定值的轨链差分 + 中性恒等/缺省非恒等 + hat 逐件链隔离）、inserts 模块内投影/恒等门 2 条、theory 预置面 2 条、dsp crusher_in_place 3 条
- [x] 盒上验证（yanshi-dev，日志 /data/workspace/shaping-verify/）：dsp 全套 31 二进制 838/0；theory 337/0；render lib+insert_chain+lane_sidechain 376/0；shaping_chain_audio 4/0；mcp --lib 751/0；mcp 普查 7 二进制 55/0（effect_contract、assertion_bounds_audit、param_bounds_guard、props_176、trap_175、fault_vocabulary、drum_generate_tool）；license_inventory --check 过；app lib drum 8/0（点名 drum_gen_ops_factory_styles_all_carry_the_full_fx_layer ok）、effect 1/0、status_bar_honesty 6/0、assertion_bounds_audit 5/0；clippy 五 crate --all-targets -D warnings 全过；fmt 过。普查同步三处已闭：r77 证据表 R77_WRITE=1 重生成（420→422／329→331／124→126）＋ count_register 三行改钉；theory `.all(` 两处改写成 family 列表断言（不进普查表）。能红自证：盒上副本变异（transient_of 恒 None、funk attack 0.45→0.30）下投影判据 2 条、funk 音频判据、theory 出厂判据全红，还原后全绿（日志 redproof-*.log）。盘面事件：app 构建一度把 /data 打满（incremental 4.7 GB），后续步 ENOSPC 失败；由本线删除自有 target/incremental 与测试二进制回收，关 incremental 后重跑全绿；完工清自有 target，/data 起止均为余 11 GB。
- [x] 提交（四笔，line/drum-shaping-wiring，不推送）

注记：开工后 main 已由 85e2953e8 进到 0f626645a（UI 补丁 Wang 裁决直合）；本线改动面与其不重叠，不 rebase，合入由父 agent 处理。
