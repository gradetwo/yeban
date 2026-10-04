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
 * 截图 (artifact):
 *   desktop-dark-zh / desktop-light-en / mobile-dark-zh / notfound-dark-zh
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
    });
    const page = await context.newPage();
    const consoleErrors = [];
    const failedRequests = [];
    page.on("console", (m) => {
      if (m.type() === "error") consoleErrors.push(m.text());
    });
    page.on("requestfailed", (r) => failedRequests.push(`${r.url()} ${r.failure()?.errorText}`));
    const url = lang ? `${base}/?lang=${lang}` : `${base}/`;
    await page.goto(url, { waitUntil: "networkidle" });
    return { context, page, consoleErrors, failedRequests };
  };

  // --- A1/A2/A3/A7: 深色自动模式 -------------------------------------------
  {
    const { context, page, consoleErrors, failedRequests } = await newPage({ dark: true });
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

  // --- A6: 移动端无横向溢出 ------------------------------------------------
  {
    const { context, page } = await newPage({ dark: true, viewport: { width: 390, height: 844 } });
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

  // --- A8: 404 页面 -------------------------------------------------------
  {
    const { context, page } = await newPage({ dark: true });
    const resp = await page.goto(`${base}/definitely-not-a-page`, { waitUntil: "networkidle" });
    const status = resp ? resp.status() : 0;
    if (status !== 404) fail("A8", `未知路径应返回 404, 实际 ${status}`);
    const notFoundText = (await page.textContent("h1")).trim();
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
