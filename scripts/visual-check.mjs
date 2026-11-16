#!/usr/bin/env node
/**
 * 浏览器端到端核验 + 截图 (scripts/visual-check.mjs)
 *
 * 为什么需要它: 静态契约检查 (check-site.mjs) 只能证明"文件和键都在",
 * 证明不了"页面在浏览器里真的长这样、主题真的切了、语言真的换了"。
 * SKILL 的诚实规则说得很直接 —— 没在真实渲染器里看过的界面, 就是没验证过。
 *
 * 这个脚本跑在 CI 的 Chromium 上 (本机不装浏览器, 遵守"本机不跑重活"的纪律),
 * 把截图作为 artifact 上传, 于是**任何人都能事后回看**, 而不是只听断言。
 *
 * 断言 (每一条都能失败):
 *   A1 无控制台错误、无失败的网络请求
 *   A2 i18n 词典成功加载, 且没有任何 data-i18n 元素仍是空的
 *   A3 默认"自动"模式跟随系统配色 (深/浅都要试)
 *   A4 手动点"浅色"/"深色"/"自动"按钮, data-theme 必须随之改变
 *   A5 切到 EN 后 <html lang> 变成 en, 且 h1 文案变成英文
 *   A6 移动端 390px 宽无横向溢出 (scrollWidth <= innerWidth + 1)
 *   A7 关键区块锚点 (#features/#stack/#roadmap/#opensource/#contact) 都在
 *   A8 404 页面可访问且语义正确
 *   A9 fr/ja/ru 渲染与词典逐字一致 (<html lang>/h1/title/meta/分享 meta/下拉值); fr/ru 全页无中文残留
 *  A10 语言检测与持久化: 无存储按浏览器语言检测; 下拉选后保持; 清存储后回到检测
 * 截图 (artifact):
 *   desktop-dark-zh / desktop-light-en / mobile-dark-zh / notfound-dark-zh
 *   desktop-dark-fr / desktop-dark-ja / desktop-dark-ru
 *
 * 用法: BASE_URL=http://127.0.0.1:8099 node scripts/visual-check.mjs
 *       (未设 BASE_URL 时脚本自己起一个静态服务器)
 */

import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFileSync, existsSync, mkdirSync, statSync } from "node:fs";
import { join, resolve, dirname, extname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PUBLIC = join(ROOT, "public");
const OUT = join(ROOT, "artifacts", "screenshots");
const failures = [];

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml; charset=utf-8",
  ".webmanifest": "application/manifest+json",
};

function serve() {
  return new Promise((ok) => {
    const server = createServer((req, res) => {
      let path = decodeURIComponent(req.url.split("?")[0]);
      if (path === "/") path = "/index.html";
      const file = resolve(PUBLIC, path.replace(/^\//, ""));

      // 目录穿越防护: 目标必须仍在 public/ 之内
      if (!file.startsWith(PUBLIC)) {
        res.writeHead(403).end("forbidden");
        return;
      }
      if (existsSync(file) && statSync(file).isFile()) {
        res.writeHead(200, { "content-type": MIME[extname(file)] || "application/octet-stream" });
        res.end(readFileSync(file));
        return;
      }
      // 模拟 Cloudflare 的 not_found_handling = "404-page"
      const fallback = join(PUBLIC, "404.html");
      if (existsSync(fallback)) {
        res.writeHead(404, { "content-type": MIME[".html"] });
        res.end(readFileSync(fallback));
      } else {
        res.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => ok({ server, port: server.address().port }));
  });
}

function fail(id, message) {
  failures.push(`[${id}] ${message}`);
}

async function main() {
  if (!existsSync(OUT)) mkdirSync(OUT, { recursive: true });

  let server = null;
  let base = process.env.BASE_URL;
  if (!base) {
    const started = await serve();
    server = started.server;
    base = `http://127.0.0.1:${started.port}`;
  }
  console.log(`base url: ${base}`);

  const browser = await chromium.launch();

  const newPage = async ({ dark, lang, viewport }) => {
    const context = await browser.newContext({
      viewport: viewport || { width: 1440, height: 900 },
      colorScheme: dark ? "dark" : "light",
      // DPR 1: 截图只用于人工/机器回看版面, 不需要 2x 体积
      deviceScaleFactor: 1,
      reducedMotion: "reduce",
      // 显式钉死 locale: CI 的 navigator.language 是 en-US, 若不钉死,
      // 名为 *-zh 的截图实际拍的是英文 (第一版就踩了这个坑)
      locale: {
        en: "en-US", fr: "fr-FR", es: "es-ES", pt: "pt-BR",
        ja: "ja-JP", ko: "ko-KR", ru: "ru-RU", de: "de-DE",
      }[lang] || "zh-CN",
    });
    const page = await context.newPage();
    const consoleErrors = [];
    const failedRequests = [];
    page.on("console", (m) => {
      if (m.type() === "error") consoleErrors.push(m.text());
    });
    // 统计主机豁免：GTM/GA 是 Wang 决定引入的第三方统计（容器 GTM-5WC4DS5B），
    // 其 beacon 在无外网的 CI 沙箱里必然失败，不计入 A1；本站自身资源失败仍判红。
    const analyticsHosts = ["www.googletagmanager.com", "www.google-analytics.com", "analytics.google.com"];
    page.on("requestfailed", (r) => {
      let host = "";
      try { host = new URL(r.url()).hostname; } catch { /* 保持空串 */ }
      if (analyticsHosts.includes(host)) return;
      failedRequests.push(`${r.url()} ${r.failure()?.errorText}`);
    });
    const url = lang ? `${base}/?lang=${lang}` : `${base}/`;
    await page.goto(url, { waitUntil: "networkidle" });
    return { context, page, consoleErrors, failedRequests };
  };

  // --- A1/A2/A3/A7: 深色自动模式 -------------------------------------------
  {
    const { context, page, consoleErrors, failedRequests } = await newPage({ dark: true, lang: "zh" });
    if (consoleErrors.length) fail("A1", `深色模式控制台错误: ${consoleErrors.join(" | ")}`);
    if (failedRequests.length) fail("A1", `深色模式请求失败: ${failedRequests.join(" | ")}`);

    const emptyI18n = await page.$$eval("[data-i18n]", (nodes) =>
      nodes.filter((n) => !n.textContent.trim()).map((n) => n.getAttribute("data-i18n"))
    );
    if (emptyI18n.length) fail("A2", `i18n 未填充的键: ${emptyI18n.join(", ")}`);

    const theme = await page.getAttribute("html", "data-theme");
    const pref = await page.getAttribute("html", "data-theme-pref");
    if (theme !== "dark" || pref !== "auto") fail("A3", `系统深色时 theme=${theme} pref=${pref}, 期望 dark/auto`);

    for (const id of ["features", "stack", "roadmap", "opensource", "contact"]) {
      if ((await page.$(`#${id}`)) === null) fail("A7", `缺少区块 #${id}`);
    }

    await page.screenshot({ path: join(OUT, "desktop-dark-zh.png"), fullPage: true });
    if (theme === "dark") {
      // A4: 手动切浅色, 必须立刻变
      await page.click('[data-theme-set="light"]');
      const afterLight = await page.getAttribute("html", "data-theme");
      if (afterLight !== "light") fail("A4", `点击浅色后 data-theme=${afterLight}`);
      await page.click('[data-theme-set="dark"]');
      const afterDark = await page.getAttribute("html", "data-theme");
      if (afterDark !== "dark") fail("A4", `点击深色后 data-theme=${afterDark}`);
      await page.click('[data-theme-set="auto"]');
      const afterAuto = await page.getAttribute("html", "data-theme");
      if (afterAuto !== "dark") fail("A4", `回到自动后 data-theme=${afterAuto}, 期望跟随系统 dark`);
    }
    await context.close();
  }

  // --- A3(浅色系统)/A5: 浅色 + 英文 ---------------------------------------
  {
    const { context, page, consoleErrors, failedRequests } = await newPage({ dark: false, lang: "en" });
    if (consoleErrors.length) fail("A1", `浅色模式控制台错误: ${consoleErrors.join(" | ")}`);
    if (failedRequests.length) fail("A1", `浅色模式请求失败: ${failedRequests.join(" | ")}`);

    const theme = await page.getAttribute("html", "data-theme");
    if (theme !== "light") fail("A3", `系统浅色时 data-theme=${theme}, 期望 light`);

    const lang = await page.getAttribute("html", "lang");
    if (lang !== "en") fail("A5", `?lang=en 时 <html lang>=${lang}`);
    const h1 = (await page.textContent("h1")).trim();
    if (!/[\x20-\x7E]/.test(h1) || /[\u4e00-\u9fff]/.test(h1)) {
      fail("A5", `?lang=en 时 h1 仍是中文: ${h1}`);
    }

    await page.screenshot({ path: join(OUT, "desktop-light-en.png"), fullPage: true });
    await context.close();
  }

  // --- A9: fr / ja / ru 与词典逐字一致 ------------------------------------
  {
    const htmlLangOf = { fr: "fr", ja: "ja", ru: "ru" };
    // 下拉里的语言自称名与作品章名是专名, 在任何语言下都保留原文, 不算残留
    const properNouns = ["中文", "English", "Français", "Español", "Português", "日本語", "한국어", "Русский", "Deutsch", "雨声太吵", "失语呢喃", "潮·失"];
    for (const lang of ["fr", "ja", "ru"]) {
      const dict = JSON.parse(readFileSync(join(PUBLIC, "assets", "i18n", `${lang}.json`), "utf8"));
      const { context, page, consoleErrors, failedRequests } = await newPage({ dark: true, lang });
      if (consoleErrors.length) fail("A9", `${lang} 控制台错误: ${consoleErrors.join(" | ")}`);
      if (failedRequests.length) fail("A9", `${lang} 请求失败: ${failedRequests.join(" | ")}`);

      const htmlLang = await page.getAttribute("html", "lang");
      if (htmlLang !== htmlLangOf[lang]) fail("A9", `?lang=${lang} 时 <html lang>=${htmlLang}, 期望 ${htmlLangOf[lang]}`);
      const h1 = (await page.textContent("h1")).trim();
      if (h1 !== dict["hero.title"]) fail("A9", `${lang} h1 与词典不一致: ${h1}`);
      const title = await page.title();
      if (title !== dict["meta.title"]) fail("A9", `${lang} <title> 与词典不一致: ${title}`);
      const desc = await page.getAttribute('meta[name="description"]', "content");
      if (desc !== dict["meta.description"]) fail("A9", `${lang} meta description 与词典不一致`);
      const ogTitle = await page.getAttribute('meta[property="og:title"]', "content");
      if (ogTitle !== dict["meta.title"]) fail("A9", `${lang} og:title 与词典不一致: ${ogTitle}`);
      const ogDesc = await page.getAttribute('meta[property="og:description"]', "content");
      if (ogDesc !== dict["meta.description"]) fail("A9", `${lang} og:description 与词典不一致`);
      const twTitle = await page.getAttribute('meta[name="twitter:title"]', "content");
      if (twTitle !== dict["meta.title"]) fail("A9", `${lang} twitter:title 与词典不一致: ${twTitle}`);
      const twDesc = await page.getAttribute('meta[name="twitter:description"]', "content");
      if (twDesc !== dict["meta.description"]) fail("A9", `${lang} twitter:description 与词典不一致`);
      const ogAlt = await page.getAttribute('meta[property="og:image:alt"]', "content");
      if (ogAlt !== dict["meta.og_image_alt"]) fail("A9", `${lang} og:image:alt 与词典不一致: ${ogAlt}`);
      const feat = await page.$eval('[data-i18n="features.title"]', (n) => n.textContent.trim());
      if (feat !== dict["features.title"]) fail("A9", `${lang} features.title 与词典不一致: ${feat}`);
      const sel = await page.$eval("[data-lang-select]", (n) => n.value);
      if (sel !== lang) fail("A9", `${lang} 时语言下拉值=${sel}, 期望 ${lang}`);

      // ja 与中文共享汉字, 逐字比对已覆盖; fr/ru 另做全页残留扫描
      if (lang !== "ja") {
        const residue = await page.evaluate((names) => {
          let text = document.body.innerText;
          for (const s of names) text = text.split(s).join("");
          const m = text.match(/[一-鿿぀-ヿ가-힯]/g);
          return m ? m.slice(0, 20).join("") : "";
        }, properNouns);
        if (residue) fail("A9", `${lang} 页面仍有中日韩文字残留: ${residue}`);
      }

      await page.screenshot({ path: join(OUT, `desktop-dark-${lang}.png`), fullPage: true });
      await context.close();
    }
  }

  // --- A10: 浏览器语言检测 + localStorage 持久化 ---------------------------
  {
    const frDict = JSON.parse(readFileSync(join(PUBLIC, "assets", "i18n", "fr.json"), "utf8"));
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
      colorScheme: "dark",
      deviceScaleFactor: 1,
      reducedMotion: "reduce",
      locale: "fr-FR",
    });
    const page = await context.newPage();
    // 1) 无 URL 参数、无存储: 必须按浏览器语言 (fr-FR) 检测出法语
    await page.goto(`${base}/`, { waitUntil: "networkidle" });
    let htmlLang = await page.getAttribute("html", "lang");
    if (htmlLang !== "fr") fail("A10", `浏览器 fr-FR 且无存储时 <html lang>=${htmlLang}, 期望 fr (自动检测)`);
    const h1 = (await page.textContent("h1")).trim();
    if (h1 !== frDict["hero.title"]) fail("A10", `自动检测后 h1 不是法语: ${h1}`);
    // 2) 下拉改成德语: 立即生效, 且去掉 URL 参数后靠 localStorage 保持
    await page.selectOption("[data-lang-select]", "de");
    await page.waitForFunction(() => document.documentElement.lang === "de");
    await page.goto(`${base}/`, { waitUntil: "networkidle" });
    htmlLang = await page.getAttribute("html", "lang");
    if (htmlLang !== "de") fail("A10", `选德语后无参数重开 <html lang>=${htmlLang}, 期望 de (localStorage 持久化)`);
    const sel = await page.$eval("[data-lang-select]", (n) => n.value);
    if (sel !== "de") fail("A10", `持久化后语言下拉值=${sel}, 期望 de`);
    // 3) 清掉存储再无参数打开: 回到浏览器语言检测 (法语)
    await page.evaluate(() => localStorage.removeItem("yeban-lang"));
    await page.goto(`${base}/`, { waitUntil: "networkidle" });
    htmlLang = await page.getAttribute("html", "lang");
    if (htmlLang !== "fr") fail("A10", `清存储后 <html lang>=${htmlLang}, 期望回到检测结果 fr`);
    await context.close();
  }

  // --- A6: 移动端无横向溢出 ------------------------------------------------
  {
    const { context, page } = await newPage({ dark: true, lang: "zh", viewport: { width: 390, height: 844 } });
    const overflow = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      innerWidth: window.innerWidth,
    }));
    if (overflow.scrollWidth > overflow.innerWidth + 1) {
      fail("A6", `390px 宽横向溢出: scrollWidth=${overflow.scrollWidth} > innerWidth=${overflow.innerWidth}`);
    }
    await page.screenshot({ path: join(OUT, "mobile-dark-zh.png"), fullPage: true });
    await context.close();
  }

  // --- A6b: 德语长复合词在移动端同样不溢出 ----------------------------------
  {
    const { context, page } = await newPage({ dark: true, lang: "de", viewport: { width: 390, height: 844 } });
    const overflow = await page.evaluate(() => ({
      scrollWidth: document.documentElement.scrollWidth,
      innerWidth: window.innerWidth,
    }));
    if (overflow.scrollWidth > overflow.innerWidth + 1) {
      fail("A6", `德语 390px 宽横向溢出: scrollWidth=${overflow.scrollWidth} > innerWidth=${overflow.innerWidth}`);
    }
    await page.screenshot({ path: join(OUT, "mobile-dark-de.png"), fullPage: true });
    await context.close();
  }

  // --- A8: 404 页面 -------------------------------------------------------
  {
    const { context, page } = await newPage({ dark: true, lang: "zh" });
    const resp = await page.goto(`${base}/definitely-not-a-page?lang=zh`, { waitUntil: "networkidle" });
    const status = resp ? resp.status() : 0;
    if (status !== 404) fail("A8", `未知路径应返回 404, 实际 ${status}`);
    const notFoundText = (await page.textContent("h1")).trim();
    // 两种语言的 notfound.title 都必须含 "404" —— 404 页面理应把 404 说出来
    if (!notFoundText.includes("404")) fail("A8", `404 页 h1 不含 404: ${notFoundText}`);
    await page.screenshot({ path: join(OUT, "notfound-dark-zh.png"), fullPage: true });
    await context.close();
  }

  await browser.close();
  if (server) server.close();

  if (failures.length) {
    console.error(`\n浏览器核验未通过 (${failures.length} 项):`);
    for (const f of failures) console.error(`  - ${f}`);
    process.exit(1);
  }
  console.log(`\n浏览器核验通过。截图已写入 ${OUT}`);
}

main().catch((err) => {
  console.error("visual-check 崩溃:", err);
  process.exit(1);
});
