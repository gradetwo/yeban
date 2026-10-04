# 品牌资产 (Brand assets)

## 目录内容

| 文件 | 说明 |
| :--- | :--- |
| `yeban.svg` | **母版**。内含 10 个独立 SVG 变体：深色/浅色 × 512/256/128/64/32 |
| `yeban-dark-{512,256,128,64,32}.svg` | 母版拆出的深色变体（透明圆角外框） |
| `yeban-light-{512,256,128,64,32}.svg` | 母版拆出的浅色变体 |
| `png/yeban-{dark,light}-{512,256,128,64,32}.png` | 光栅化产物，供 favicon / 触屏图标 / 文档插图 |

## 母版为什么能安全拆分

每个变体内部的 `<defs>` id 都带尺寸与主题前缀（`d512-bg`、`l32-moon`…），因此十个变体之间
**没有 id 冲突**，可以共存于一个文件，也可以逐个拆出来单独使用。拆分与光栅化都由
`scripts/brand/render-logo.sh` 完成，不手改。

## 品牌色（从母版 SVG 中提取，非目测）

| 用途 | 深色模式 | 浅色模式 |
| :--- | :--- | :--- |
| 背景 | 径向渐变 `#151D38` → `#060A14` | `#FDFCFA` / `#F0EBE3` |
| 主金（月牙与声波） | 线性渐变 `#F7E6B0` → `#E2C77E` → `#B8933E` | 同左 |
| 文字/墨色 | `#F0EBE3` | `#1A2340` / `#2A3A5C` |
| 次级灰蓝 | `#5A6B8A` | `#8A7E6E` |

官网（`website` 分支的 `yeban.wangda.today`）的主题变量直接引用上表，
保证站点与图标同源。

## 重新生成

```bash
bash scripts/brand/render-logo.sh      # 需要 rsvg-convert (brew install librsvg)
```

## 许可

品牌标识为夜半项目所有，使用规则见仓库根目录 [`TRADEMARK.md`](../../TRADEMARK.md)。
`TRADEMARK.md` 属于治理文件，Agent 不得修改（`AGENTS.md` §2 红线 1）。
