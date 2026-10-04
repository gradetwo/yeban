/* 外观主题: 自动 / 浅色 / 深色
 *
 * 语义约定 (与 index.html 的内联首屏脚本、CSS 选择器三方一致):
 *   <html data-theme-pref="auto|light|dark">  用户的**选择**
 *   <html data-theme="dark|light">            解析后的**实际外观** (CSS 只看这个)
 *
 * "自动" 跟随 prefers-color-scheme, 并且在系统切换时**实时**跟随 —— 不是只在加载时读一次。
 */
(function () {
  "use strict";

  var STORAGE_KEY = "yeban-theme";
  var root = document.documentElement;
  var mql = window.matchMedia("(prefers-color-scheme: dark)");

  function readPref() {
    try {
      var v = localStorage.getItem(STORAGE_KEY);
      return v === "light" || v === "dark" || v === "auto" ? v : "auto";
    } catch (e) {
      return "auto";
    }
  }

  function writePref(pref) {
    try { localStorage.setItem(STORAGE_KEY, pref); } catch (e) { /* 隐私模式下忽略 */ }
  }

  function resolve(pref) {
    if (pref === "auto") return mql.matches ? "dark" : "light";
    return pref;
  }

  function apply(pref) {
    root.setAttribute("data-theme-pref", pref);
    root.setAttribute("data-theme", resolve(pref));
    var buttons = document.querySelectorAll("[data-theme-set]");
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].setAttribute("aria-pressed", String(buttons[i].getAttribute("data-theme-set") === pref));
    }
  }

  function set(pref) {
    writePref(pref);
    apply(pref);
  }

  // 系统主题变化时, 只有在"自动"模式下才跟随
  var onSystemChange = function () {
    if (root.getAttribute("data-theme-pref") === "auto") apply("auto");
  };
  if (typeof mql.addEventListener === "function") mql.addEventListener("change", onSystemChange);
  else if (typeof mql.addListener === "function") mql.addListener(onSystemChange);

  // 暴露给 site.js 做事件绑定 (不用内联 onclick, 以免违反 CSP 友好性)
  window.YebanTheme = { apply: apply, set: set, read: readPref };

  apply(readPref());
})();
