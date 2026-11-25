# 《编曲与配器手册》截图清单（汇总去重）

全册中英双版共用同一张界面图：同一 `\screenshot{name}` 在 `zh/` 与 `en/` 各出现一次，按图名去重后共 **11 张**。管理侧按本表统一拍摄，回填到各语言 `figures/` 目录；图名与正文 `\screenshot` 占位完全一致。

| # | 图名 | 章节 | 要拍的界面状态 | 拍摄要点 |
| :-- | :-- | :-- | :-- | :-- |
| 1 | `arrangement-demo-six-tracks` | 第 1 章 | 演示工程六条普通轨与主总线 | 六轨与 Master 可见；逐轨 Solo/Mute 状态可辨；不裁掉轨名 |
| 2 | `arrangement-song-structure-overview` | 第 2 章 | 编曲区小节标尺/段落标记与全曲总览面板 | 同一屏看到段落边界和能量分布；段落标记不被面板遮住 |
| 3 | `arrangement-rhythm-section-groove` | 第 3 章 | 鼓、贝斯、伴奏三轨的八拍骨架与混音台低频电平 | 强拍/反拍/留空关系可读；低频电平可见 |
| 4 | `arrangement-harmony-layer-voicing` | 第 4 章 | 和弦轨、钢琴卷帘铺底/琶音排列、混音台中频电平 | 顶音走向与声部让位可读；中频电平可见 |
| 5 | `arrangement-melody-counterpoint` | 第 5 章 | 主旋律/副旋律两轨、钢琴卷帘与力度泳道 | 两条线音区错开；目标音与句尾呼吸可读 |
| 6 | `arrangement-string-voicing` | 第 6 章 | 五条弦乐 MIDI 轨、钢琴卷帘与自动化泳道 | 第一小提琴/第二小提琴/中提琴/大提琴/低音提琴分层可见；内声部不遮主旋律 |
| 7 | `arrangement-winds-brass` | 第 7 章 | 木管与铜管分轨、钢琴卷帘与自动化泳道 | 木管答句短、铜管重音短；二者进出位置可读 |
| 8 | `arrangement-electronic-layers` | 第 8 章 | 低频/主体/空气/节奏四条 MIDI 轨加一条采样轨 | 钢琴卷帘与自动化泳道展示副歌层次；采样轨与 MIDI 轨分开 |
| 9 | `arrangement-vocal-stack` | 第 9 章 | 主唱/双唱/和声/叠唱四条音频轨 | 片段进出与片段静音状态可读；混音台声像位置可见 |
| 10 | `arrangement-walkthrough-folk` | 第 10 章 | 民谣十六小节工程六轨 | 主唱/木吉他/贝斯/轻鼓/弦乐 Pad/双唱；按段落表可见主歌到副歌进出 |
| 11 | `arrangement-walkthrough-pop` | 第 10 章 | 流行十六小节工程八轨 | 主唱/双唱/和声/鼓/贝斯/Pad/Pluck/铜管；第 9 小节铜管短重音与副歌层次可见 |

## 不计入清单的文件

- `ch01-sample.tex` 里的 `main-window-overview`：该文件是骨架写作样例，未接入 `main.tex`，其截图占位不计入交付清单。
- 正文中的 `\screenshot{图名}` 说明文字不是图名，不参与去重。

## 去重结论

- 中英各 11 处 `\screenshot`，图名一一对应；按图名去重后 11 张。
- 无同名不同义的截图；无同义不同名（章节差异均保留不同图名）。
