# 夜半 (Yeban) 官网 — yeban.wangda.today

本分支（`website`，orphan 分支）**只**承载官网静态站点。它刻意与 `main` 上的 Rust 工作区分离：
部署产物不需要任何 Rust 工具链，站点改动也不会污染 DAW 仓库的历史。

## 结构

```text
.
├── public/                     # ← 部署根目录（wrangler [assets] directory 指向这里）
│   ├── index.html              # 单页站点：双语 + 深浅色自适应
│   ├── 404.html                # 自定义 404（not_found_handling = "404-page"）
│   ├── robots.txt / sitemap.xml / site.webmanifest
│   └── assets/
│       ├── css/site.css        # 全部样式（无框架、无外部字体）
│       ├── js/theme.js         # 自动/浅色/深色，跟随系统实时切换
│       ├── js/i18n.js          # 中英切换（?lang=zh|en + localStorage）
│       ├── js/site.js          # 控件装配
│       ├── i18n/{zh,en}.json   # 两份词典，键集必须完全相同（门禁 C1 强制）
│       └── brand/              # 来自 main 的 assets/brand，另含 favicon/OG 图
├── scripts/check-site.mjs      # 站点静态契约检查（10 组判据，本机可跑）
├── wrangler.toml               # Cloudflare Workers 配置
└── .github/workflows/site-deploy.yml
```

## 设计要点

| 需求 | 实现 |
| :--- | :--- |
| 中英双语可切换 | `data-i18n` 标注 + 两份 JSON 词典；`?lang=en` 可直接分享；切换后同步 `<html lang>` 供读屏软件使用 |
| 深浅色可切换，默认自动 | `<html data-theme-pref>`（用户选择）+ `<html data-theme>`（解析结果）；"自动"跟随 `prefers-color-scheme` 并**实时**响应系统切换；首屏内联脚本消除闪白 |
| 无第三方依赖 | 无框架、无外部字体、无分析脚本、无 Cookie（门禁 C10 会拦下白名单外的外部主机） |
| 渐进增强 | HTML 内已写中文原文；即使词典请求失败或 JS 被禁用，页面依然完整可读 |
| 深浅双 logo | 品牌母版里深/浅两套图标，由 CSS 按 `data-theme` 切换，避免深色背景上放浅色底图标 |

品牌色与图标来源见 `main` 分支的 `assets/brand/README.md`；本分支的 `public/assets/brand/` 是它的部署副本。

## 本地开发与校验

```bash
node scripts/check-site.mjs            # 站点契约检查（零依赖，无需联网）
wrangler deploy --dry-run              # 校验 wrangler.toml 与 assets 目录（不需要凭据）
python3 -m http.server -d public 8080  # 任意静态服务器预览
```

## 部署

`.github/workflows/site-deploy.yml` 在**推送到本分支**时自动执行：
先跑 `check-site.mjs`，再 `wrangler deploy`。

需要在仓库 Secrets 里配置（未配置时部署步骤会**优雅跳过**并给出提示，不会报红）：

| Secret | 用途 |
| :--- | :--- |
| `CLOUDFLARE_API_TOKEN` | 权限：Workers Scripts:Edit |
| `CLOUDFLARE_ACCOUNT_ID` | 账号 ID |

自定义域 `yeban.wangda.today`：在 Cloudflare 控制台接入该域后，取消 `wrangler.toml` 里
`[[routes]]` 的注释再部署即可。未配置时会先发到 `<name>.<subdomain>.workers.dev`。

## 许可

本站点是**夜半 (Yeban) 项目**的一部分，整体以 **GNU GPLv3**（附 GPLv3 §7 的 CLAP 插件例外条款）发布。
完整的许可文本与治理文件在 `main` 分支的仓库根目录（`LICENSE` / `LEGAL.md` / `TRADEMARK.md`）：

```bash
git show main:LICENSE | head -20     # 或直接在 GitHub 上切到 main 分支查看
```

页面页脚也明确声明了这一许可。本分支是 orphan 分支（只承载站点），因此不重复存放许可全文 ——
**唯一权威文本始终是 `main` 分支的 `LICENSE`**，避免出现两份可能不一致的副本。

"夜半 / Yeban" 的标识使用规则见 `main` 分支的 `TRADEMARK.md`。

## 门禁能失败吗

`scripts/check-site.mjs` 的每组判据都写了"怎么才会红"，并且经过实测：

- 删掉 `en.json` 里的任意一个键 → C1 红
- 把 HTML 里的 `data-i18n="nav.features"` 改成不存在的键 → C2 红
- 删掉一个主题/语言切换按钮 → C6 红
- 引用一张不存在的图片 → C4 红

详见仓库 `main` 分支的 `docs/DEVELOPMENT_LEDGER.md`（账本会记录官网这一版的判据实证）。
