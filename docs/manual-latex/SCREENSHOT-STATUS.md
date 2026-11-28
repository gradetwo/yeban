# 截图入库状态（T-FIX-1040）

基线：`line/incoming-doc-assemble-all` @ `c9d79ab147012a4ba4bfbdaf64854d5c199df4bd`。

零图章节：22 个文件已补 22 个 `tikzpicture`（任务点名的传奇乐器 2 个文件 + 8 个曲风 ×2 + 曲风总览 2 + 混音进阶 2；后两者为相邻零图章节）。

## 汇总

| 册 | 图名需求 | 实图入库 | 阻塞/不可达 |
|---|---:|---:|---:|
| user-guide | 26 | 20 | 6 |
| instruments | 15 | 13 | 2 |
| music-theory | 9 | 9 | 0 |
| arrangement | 11 | 11 | 0 |
| mixing | 23 | 21 | 2 |

总计：84 个图名；74 张实图入库；10 项不可达并逐项登记。所有入库图按 SHA-256 去重，74 个哈希两两不同。

## 入库清单

### user-guide

| 图名 | 尺寸 | SHA-256（前 16） |
|---|---:|---|
| `ch01-export-dialog` | 1920×1080 | `f656d17827a556cc…` |
| `ch01-first-clip` | 1920×1080 | `0cbc582f170c6e61…` |
| `ch01-main-window` | 1920×1080 | `1bd1f22596df019d…` |
| `ch01-onboarding-tour` | 1920×1080 | `5b2b28e0f47206d9…` |
| `ch02-file-menu` | 1920×1080 | `5ab966717e269044…` |
| `ch02-new-project-details` | 1920×1080 | `99980207448f26ca…` |
| `ch02-startup-chooser` | 1920×1080 | `a580931810408bdf…` |
| `ch02-template-dialog` | 1920×1080 | `ef6a9ad419aad79f…` |
| `ch02-undo-tree` | 1920×1080 | `c6d8a6f3a9c6bc96…` |
| `ch03-clip-matrix` | 1920×1080 | `e3e3087f6bbeac91…` |
| `ch03-track-header` | 1200×410 | `67d801c8d4629042…` |
| `ch04-humanize-dialog` | 1920×1080 | `c8c1d27a775befcc…` |
| `ch04-roll-overview` | 1920×1080 | `9d6d785b183fa4b9…` |
| `ch05-audio-clip-edges` | 1920×1080 | `33b4122894c34e45…` |
| `ch05-record-panel` | 1920×1080 | `0121f6d0340a5f3f…` |
| `ch06-automation-lanes` | 1920×1080 | `db9646e72d060ed9…` |
| `ch07-control-room` | 1920×1080 | `b0258ad9f92f3efc…` |
| `ch07-mixer-console` | 1920×1080 | `8dcc146abb8a0d32…` |
| `ch08-export-wav-dialog` | 1920×1080 | `c6829b2d8203acd8…` |
| `ch09-shortcuts-panel` | 1920×1080 | `625436ca5acd9d41…` |

### instruments

| 图名 | 尺寸 | SHA-256（前 16） |
|---|---:|---|
| `drum-clip-piano-roll` | 1920×1080 | `288451bc2be43dcc…` |
| `drum-gen-dialog` | 1920×1080 | `272c0bceed05acb3…` |
| `drum-track-device-rack` | 1920×1080 | `7305ba0879e62177…` |
| `factory-preset-picker` | 1920×1080 | `24d78c8126f4de9c…` |
| `instrument-picker` | 1920×1080 | `6046dbb60e75b460…` |
| `instrument-rack-synth-presets` | 1920×1080 | `be6c949415c83e23…` |
| `plugin-panel-load` | 1920×1080 | `6b9b76b45f8d8e3e…` |
| `plugin-panel-scan` | 1920×1080 | `09684a1cca822014…` |
| `sample-library-picker` | 1920×1080 | `600cffe47310ff5a…` |
| `synth-bass-recipe` | 1920×1080 | `eb33299139b6f162…` |
| `synth-device-params` | 1920×1080 | `91838bccad90726e…` |
| `synth-lead-recipe` | 1920×1080 | `bf0093492086960e…` |
| `synth-pad-recipe` | 1920×1080 | `f396167ade63e8ae…` |

### music-theory

| 图名 | 尺寸 | SHA-256（前 16） |
|---|---:|---|
| `arrangement-aaba-structure` | 1920×1080 | `90ce78a7b6cfe611…` |
| `piano-roll-c-major-triad` | 1920×1080 | `9eb567df882f7d8d…` |
| `piano-roll-four-bar-question-answer` | 1920×1080 | `4d6095efb1fb65bb…` |
| `piano-roll-ii-v-i` | 1920×1080 | `319dc7f21d14504c…` |
| `piano-roll-interval-two-notes` | 1920×1080 | `670acd761ac3220a…` |
| `piano-roll-pitch-labels` | 1920×1080 | `9aeaefa6e8bdaf80…` |
| `piano-roll-rhythm-grid` | 1920×1080 | `31b3e0291196470b…` |
| `piano-roll-scale-ladder` | 1920×1080 | `17394832f1f03ae6…` |
| `piano-roll-staff-mapping` | 1920×1080 | `73bf81a15b454c05…` |

### arrangement

| 图名 | 尺寸 | SHA-256（前 16） |
|---|---:|---|
| `arrangement-demo-six-tracks` | 1920×1080 | `d96607bdc14a0dab…` |
| `arrangement-electronic-layers` | 1920×1080 | `4fceebbaf1d9b2c4…` |
| `arrangement-harmony-layer-voicing` | 1920×1080 | `fe1c34252e9cedfd…` |
| `arrangement-melody-counterpoint` | 1920×1080 | `2b4d6f0988cf2919…` |
| `arrangement-rhythm-section-groove` | 1920×1080 | `57e2674153343011…` |
| `arrangement-song-structure-overview` | 1920×1080 | `5b2b7fdedbc351c2…` |
| `arrangement-string-voicing` | 1920×1080 | `b7327d36625ee63e…` |
| `arrangement-vocal-stack` | 1920×1080 | `87351d8b3b22ffde…` |
| `arrangement-walkthrough-folk` | 1920×1080 | `b18283f78c4382ff…` |
| `arrangement-walkthrough-pop` | 1920×1080 | `2021af246909f404…` |
| `arrangement-winds-brass` | 1920×1080 | `5c4fdcd6996d1f4a…` |

### mixing

| 图名 | 尺寸 | SHA-256（前 16） |
|---|---:|---|
| `automation-device-param` | 1920×1080 | `9aaefbd456545f80…` |
| `automation-lane-overview` | 1920×1080 | `92d9c180e698bbec…` |
| `automation-point-edit` | 1920×1080 | `4589ab9e374f784b…` |
| `control-room-mono-side` | 1920×1080 | `935bc8e70c18c459…` |
| `control-room` | 1920×1080 | `24da064c00d07925…` |
| `device-rack-clipper-crusher` | 1920×1080 | `fd674621b9c50736…` |
| `device-rack-compressor` | 1920×1080 | `315b22145bef4cf4…` |
| `device-rack-eq-curve` | 1920×1080 | `078a0b8bac02e2d9…` |
| `device-rack-reverb-installed` | 1920×1080 | `809238da464b274b…` |
| `device-rack-strip-width` | 1920×1080 | `a25cf5d33215adc8…` |
| `device-rack-tape` | 1920×1080 | `135fe1d2cf93ada6…` |
| `export-wav-dialog` | 1920×1080 | `3b7397f6d4c99cdc…` |
| `master-bus-chain` | 1920×1080 | `04e213039cbd0d1f…` |
| `mixer-channel-strip` | 1920×1080 | `af3aceb6c8809ec4…` |
| `mixer-console` | 1920×1080 | `4db990bd1a94f3c2…` |
| `mixer-pan-strip` | 1920×1080 | `b06ce328c61a5643…` |
| `walkthrough-automation` | 1920×1080 | `df72fb2e3859afaa…` |
| `walkthrough-compressor` | 1920×1080 | `db39a875ed283d3a…` |
| `walkthrough-console` | 1920×1080 | `6524e4fdea202563…` |
| `walkthrough-eq-low` | 1920×1080 | `5a444f3e4c5e9aaa…` |
| `walkthrough-export` | 1920×1080 | `9b0d0840ed5bda05…` |

## 不可达项

- `user-guide/ch03-multi-select` — 未拍到：源码有 marquee，但 GUI 拖动无法稳定产生选择矩形，未用假图
- `user-guide/ch05-asset-pool` — blocked：资产池面板位于隐藏的 MixerConsole 内
- `user-guide/ch07-sends` — blocked：SND A/B 为硬编码占位，完整发送条未接入
- `user-guide/ch08-stems-toast` — blocked：无可见 stems UI / 完成提示
- `user-guide/ch10-mcp-modes` — blocked：产品内无 MCP 模式面板
- `user-guide/ch11-silence-checklist` — blocked：无对应帮助章节/静音检查清单界面
- `instruments/device-rack-plugin-instrument` — blocked：Serum 路径不存在，装载失败 os error 3
- `instruments/sfz-load-report` — blocked：无装载回执面板，仅 CLI/测试
- `mixing/bus-aux-send` — blocked：发送条位于 if false 兼容块，未接入产品
- `mixing/send-aux-reverb` — blocked：发送条未接入产品

编译时 `\screenshot` 对缺失图输出 `!! yeban-manual: MISSING SCREENSHOT` 与 `PackageWarning`，并保留灰框占位；不复制其他图充数。
