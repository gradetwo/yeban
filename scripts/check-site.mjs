#!/usr/bin/env node
/**
 * 官网静态契约检查 (scripts/check-site.mjs)
 *
 * 这是**官网自己的门禁**, 跑在 CI 的 Site workflow 里, 也在本机可跑 (纯 Node, 无依赖, 不联网)。
 * 它的存在理由与 SKILL 规则 2 一致: 判据必须能失败。每一条检查都在下面注明"怎么才会红"。
 *
 * 检查项:
 *   C1 两份词典 JSON 可解析, 且**键集完全相同**        -> 单边删/加一个键就红
 *   C2 HTML 里每个 data-i18n / data-i18n-attr 的键都在两份词典里存在 -> 打错键名就红
 *   C3 词典里没有未被使用的键 (死键)                    -> 删掉 HTML 用法留下词典条目就红
 *   C4 所有站内 href/src 指向的文件真实存在于 public/    -> 引用了不存在的图片/CSS 就红
 *   C5 站内链接不含 http:// 明文                        -> 写 http:// 就红
 *   C6 主题与语言控件、跳过链接、语义锚点齐全 (DOM 契约) -> 删掉一个切换按钮就红
 *   C7 必备文件存在且格式合法: 404.html / robots.txt / sitemap.xml / site.webmanifest / wrangler.toml
 *   C8 <html lang>、hreflang alternates、canonical 存在
 *   C9 品牌图标文件存在且非空 (favicon / apple-touch-icon / 两种主题的 64 与 512)
 *  C10 页面不含 0.0.0.0, 不含内联第三方域名 (除 github.com / wangda.today 白名单)
 *
 * 用法: node scripts/check-site.mjs
 * 退出码: 0 全过, 1 有失败项。
 */

import { readFileSync, existsSync, statSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PUBLIC = join(ROOT, "public");
const failures = [];
const notes = [];

function fail(check, message) {
  failures.push(`[${check}] ${message}`);
}

function readJson(rel) {
  const path = join(ROOT, rel);
  if (!existsSync(path)) {
    fail(rel.endsWith("toml") ? "C7" : "C1", `缺少文件: ${rel}`);
    return null;
  }
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    fail("C1", `${rel} 不是合法 JSON: ${err.message}`);
    return null;
  }
}

// ---------------------------------------------------------------- C1/C2/C3

const zh = readJson("public/assets/i18n/zh.json");
const en = readJson("public/assets/i18n/en.json");
const htmlFiles = ["index.html", "404.html"];
const htmlByFile = {};
for (const f of htmlFiles) {
  const path = join(PUBLIC, f);
  if (!existsSync(path)) {
    fail("C7", `缺少 public/${f}`);
    htmlByFile[f] = "";
  } else {
    htmlByFile[f] = readFileSync(path, "utf8");
  }
}
const html = htmlByFile["index.html"];
// i18n 键的使用是**跨页面**的: 404.html 用 notfound.*, 首页用其余键。
const allHtml = htmlFiles.map((f) => htmlByFile[f]).join("\n");

if (zh && en) {
  const zhKeys = new Set(Object.keys(zh));
  const enKeys = new Set(Object.keys(en));
  for (const k of zhKeys) if (!enKeys.has(k)) fail("C1", `en.json 缺少键: ${k}`);
  for (const k of enKeys) if (!zhKeys.has(k)) fail("C1", `zh.json 缺少键: ${k}`);
  if (zhKeys.size === 0) fail("C1", "zh.json 是空的");

  // 空译文也算缺陷: 切换语言后会出现空白
  for (const [name, dict] of [["zh", zh], ["en", en]]) {
    for (const [k, v] of Object.entries(dict)) {
      if (typeof v !== "string" || v.trim() === "") fail("C1", `${name}.json 的 \`${k}\` 是空字符串`);
    }
  }

  const used = new Set();
  for (const m of allHtml.matchAll(/data-i18n="([^"]+)"/g)) used.add(m[1]);
  for (const m of allHtml.matchAll(/data-i18n-attr="([^"]+)"/g)) {
    for (const pair of m[1].split(",")) {
      const parts = pair.split(":");
      if (parts.length !== 2) fail("C2", `data-i18n-attr 写法非法: "${m[1]}"`);
      else used.add(parts[1].trim());
    }
  }
  for (const m of allHtml.matchAll(/data-i18n-lang="([^"]+)"/g)) used.add(m[1]);

  for (const k of used) {
    if (!zhKeys.has(k)) fail("C2", `HTML 用到 \`${k}\`, 但 zh.json 里没有`);
    if (!enKeys.has(k)) fail("C2", `HTML 用到 \`${k}\`, 但 en.json 里没有`);
  }
  for (const k of zhKeys) {
    if (!used.has(k)) fail("C3", `词典里的 \`${k}\` 在 HTML 里从未被使用 (死键)`);
  }
  notes.push(`i18n: 词典各 ${zhKeys.size} 键, ${htmlFiles.join("+")} 共用到 ${used.size} 键`);
}

// ------------------------------------------------------------------- C4/C5

function checkReference(rawRef, where) {
  if (!rawRef) return;
  if (/^(https?:|mailto:|#|data:)/.test(rawRef)) {
    if (rawRef.startsWith("http://")) fail("C5", `${where} 使用了明文 http:// : ${rawRef}`);
    return;
  }
  const clean = rawRef.split("?")[0].split("#")[0];
  if (clean === "" || clean === "/") return;
  const target = join(PUBLIC, clean.replace(/^\//, ""));
  if (!existsSync(target)) fail("C4", `${where} 引用了不存在的站内路径: ${rawRef}`);
}

for (const [f, text] of Object.entries(htmlByFile)) {
  for (const m of text.matchAll(/(?:href|src)="([^"]+)"/g)) checkReference(m[1], f);
}

// --------------------------------------------------------------------- C6

const domContract = [
  ['id="main"', "缺少主内容锚点 #main (跳过链接会失效)"],
  ['class="skip-link"', "缺少跳过链接 (UI-A11Y 精神)"],
  ["data-theme-set=\"auto\"", "缺少主题-自动按钮"],
  ["data-theme-set=\"light\"", "缺少主题-浅色按钮"],
  ["data-theme-set=\"dark\"", "缺少主题-深色按钮"],
  ['data-lang-set="zh"', "缺少语言-中文按钮"],
  ['data-lang-set="en"', "缺少语言-英文按钮"],
  ['data-i18n="brand.name"', "缺少品牌名 (i18n 契约锚点)"],
  ['id="features"', "缺少 #features 区块"],
  ['id="stack"', "缺少 #stack 区块"],
  ['id="roadmap"', "缺少 #roadmap 区块"],
  ['id="opensource"', "缺少 #opensource 区块"],
  ['id="contact"', "缺少 #contact 区块"],
  ['href="mailto:yeban@wangda.today"', "缺少联系邮箱链接"],
  ['href="https://github.com/gradetwo/yeban"', "缺少仓库链接"],
];
for (const [needle, message] of domContract) {
  if (!html.includes(needle)) fail("C6", message);
}

// --------------------------------------------------------------------- C7

const requiredFiles = ["public/404.html", "public/robots.txt", "public/sitemap.xml", "public/site.webmanifest", "wrangler.toml"];
for (const rel of requiredFiles) {
  if (!existsSync(join(ROOT, rel))) fail("C7", `缺少必备文件: ${rel}`);
}

// sitemap.xml 必须至少包含首页, 且是 https
if (existsSync(join(PUBLIC, "sitemap.xml"))) {
  const sitemap = readFileSync(join(PUBLIC, "sitemap.xml"), "utf8");
  if (!sitemap.includes("<urlset")) fail("C7", "sitemap.xml 缺少 <urlset>");
  if (!sitemap.includes("https://yeban.wangda.today/")) fail("C7", "sitemap.xml 没有收录首页");
}
// robots.txt 应指向 sitemap
if (existsSync(join(PUBLIC, "robots.txt"))) {
  const robots = readFileSync(join(PUBLIC, "robots.txt"), "utf8");
  if (!robots.includes("Sitemap:")) fail("C7", "robots.txt 没有 Sitemap 行");
}
// webmanifest 必须是合法 JSON 且含必需字段
const manifest = readJson("public/site.webmanifest");
if (manifest) {
  for (const field of ["name", "short_name", "start_url", "icons"]) {
    if (!(field in manifest)) fail("C7", `site.webmanifest 缺少字段: ${field}`);
  }
  if (Array.isArray(manifest.icons)) {
    for (const icon of manifest.icons) checkReference(icon.src, "site.webmanifest icons");
  }
}
// wrangler.toml 必须把 assets 目录指向 ./public
if (existsSync(join(ROOT, "wrangler.toml"))) {
  const wrangler = readFileSync(join(ROOT, "wrangler.toml"), "utf8");
  if (!/\[assets\]/.test(wrangler)) fail("C7", "wrangler.toml 缺少 [assets] 段");
  if (!/directory\s*=\s*"\.\/public"/.test(wrangler)) fail("C7", 'wrangler.toml 的 [assets] directory 必须指向 "./public"');
}

// --------------------------------------------------------------------- C8

if (!/<html[^>]+lang="/.test(html)) fail("C8", "<html> 缺少 lang 属性");
if (!/hreflang="en"/.test(html)) fail("C8", "缺少 hreflang=en 的 alternate 链接");
if (!/hreflang="x-default"/.test(html)) fail("C8", "缺少 hreflang=x-default");
if (!/rel="canonical"/.test(html)) fail("C8", "缺少 canonical");
if (!/name="viewport"/.test(html)) fail("C8", "缺少 viewport (移动端会缩放错乱)");

// --------------------------------------------------------------------- C9

const brandAssets = [
  "public/assets/brand/yeban-dark-64.svg",
  "public/assets/brand/yeban-light-64.svg",
  "public/assets/brand/yeban-dark-512.svg",
  "public/assets/brand/yeban-light-512.svg",
  "public/assets/brand/png/yeban-dark-32.png",
  "public/assets/brand/apple-touch-icon.png",
  "public/assets/brand/favicon.ico",
  "public/assets/brand/og.png",
];
for (const rel of brandAssets) {
  const path = join(ROOT, rel);
  if (!existsSync(path)) fail("C9", `缺少品牌资产: ${rel}`);
  else if (statSync(path).size === 0) fail("C9", `品牌资产是空文件: ${rel}`);
}

// -------------------------------------------------------------------- C10

if (html.includes("0.0.0.0")) fail("C10", "页面出现 0.0.0.0");
const allowedHosts = ["github.com", "yeban.wangda.today", "schema.org", "www.w3.org"];
for (const m of html.matchAll(/https?:\/\/([^/"'\s]+)/g)) {
  const host = m[1];
  if (!allowedHosts.some((h) => host === h || host.endsWith("." + h))) {
    fail("C10", `引用了白名单外的外部主机: ${host} (静态站点不应引入第三方依赖)`);
  }
}
// CSS/JS 里也不应出现外链
for (const rel of ["public/assets/css/site.css", "public/assets/js/site.js", "public/assets/js/theme.js", "public/assets/js/i18n.js"]) {
  const p = join(ROOT, rel);
  if (!existsSync(p)) continue;
  const text = readFileSync(p, "utf8");
  for (const m of text.matchAll(/https?:\/\/([^/"'\s)]+)/g)) {
    const host = m[1];
    if (!allowedHosts.some((h) => host === h || host.endsWith("." + h))) {
      fail("C10", `${rel} 引用了外部主机: ${host}`);
    }
  }
}

// ------------------------------------------------------------------ report

for (const n of notes) console.log(`note: ${n}`);
if (failures.length) {
  console.error(`\n站点契约检查未通过 (${failures.length} 项):`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("\n站点契约检查通过。");
