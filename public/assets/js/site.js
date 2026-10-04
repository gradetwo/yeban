/* 页面交互装配 (仅此而已: 主题、语言、年份)
 *
 * 明确不做的事:
 *   - 不加载任何第三方脚本/字体/分析;
 *   - 不写 Cookie (主题与语言只存在 localStorage, 且取不到时静默降级);
 *   - 不做滚动监听/动画特效 (静态站点的价值在于快与稳)。
 */
(function () {
  "use strict";

  function ready(fn) {
    if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", fn);
    else fn();
  }

  ready(function () {
    // 1. 主题切换 (auto / light / dark)
    var themeButtons = document.querySelectorAll("[data-theme-set]");
    for (var i = 0; i < themeButtons.length; i++) {
      themeButtons[i].addEventListener("click", function (event) {
        var pref = event.currentTarget.getAttribute("data-theme-set");
        if (window.YebanTheme) window.YebanTheme.set(pref);
      });
    }

    // 2. 语言切换 (zh / en)
    var langButtons = document.querySelectorAll("[data-lang-set]");
    for (var j = 0; j < langButtons.length; j++) {
      langButtons[j].addEventListener("click", function (event) {
        var lang = event.currentTarget.getAttribute("data-lang-set");
        if (!window.YebanI18n) return;
        window.YebanI18n.set(lang);
        window.YebanI18n.syncUrl(lang);
      });
    }

    // 3. 页脚年份 (不写死, 免得每年都要改一次代码)
    var year = document.getElementById("year");
    if (year) year.textContent = String(new Date().getFullYear());

    // 4. 若 URL 带了 ?lang=en, 语言按钮的状态要与之一致 (i18n.js 会设, 这里只兜底)
    try {
      var urlLang = new URLSearchParams(window.location.search).get("lang");
      if (urlLang && window.YebanI18n) window.YebanI18n.set(urlLang);
    } catch (e) { /* 忽略 */ }
  });
})();
