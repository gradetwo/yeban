/* 页面交互装配 (仅此而已: 主题、语言、年份、截图轮播)
 *
 * 明确不做的事:
 *   - 不加载任何第三方脚本/字体/分析;
 *   - 不写 Cookie (主题与语言只存在 localStorage, 且取不到时静默降级);
 *   - 不做滚动监听/动画特效 (静态站点的价值在于快与稳)。截图轮播是唯一的
 *     动态区: 只在进入视口时自动播放, 悬停/聚焦时暂停, 系统要求减少动态
 *     时不自动播放; 手动切换 (按钮/圆点/方向键/滑动) 始终可用。
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

    // 2. 语言切换 (下拉选择 9 种语言)
    var langSelects = document.querySelectorAll("[data-lang-select]");
    for (var j = 0; j < langSelects.length; j++) {
      langSelects[j].addEventListener("change", function (event) {
        var lang = event.currentTarget.value;
        if (!window.YebanI18n) return;
        window.YebanI18n.set(lang);
        window.YebanI18n.syncUrl(lang);
      });
    }

    // 3. 页脚年份 (不写死, 免得每年都要改一次代码)
    var year = document.getElementById("year");
    if (year) year.textContent = String(new Date().getFullYear());

    // 4. 若 URL 带了 ?lang=fr, 语言下拉的状态要与之一致 (i18n.js 会设, 这里只兜底)
    try {
      var urlLang = new URLSearchParams(window.location.search).get("lang");
      if (urlLang && window.YebanI18n) window.YebanI18n.set(urlLang);
    } catch (e) { /* 忽略 */ }

    // 5. 界面截图轮播: 自动轮播 + 手动切换 (按钮/圆点/方向键/滑动)
    var carousel = document.querySelector("[data-carousel]");
    if (carousel) {
      var track = carousel.querySelector("[data-carousel-track]");
      var slides = carousel.querySelectorAll(".carousel-slide");
      var dots = carousel.querySelectorAll("[data-carousel-dot]");
      var prevBtn = carousel.querySelector("[data-carousel-prev]");
      var nextBtn = carousel.querySelector("[data-carousel-next]");
      var index = 0;
      var timer = null;
      var visible = false;
      var DELAY = 5000;
      var reduceMotion = Boolean(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);

      // 轮播图改成立即加载: 懒加载按版面位置判定, 滑出画布外的图可能不触发, 切换时白屏
      var imgs = carousel.querySelectorAll("img");
      for (var a = 0; a < imgs.length; a++) imgs[a].loading = "eager";

      function go(next) {
        index = ((next % slides.length) + slides.length) % slides.length;
        track.style.transform = "translateX(" + (-index * 100) + "%)";
        for (var d = 0; d < dots.length; d++) {
          if (d === index) dots[d].setAttribute("aria-current", "true");
          else dots[d].removeAttribute("aria-current");
        }
      }
      function stop() {
        if (timer) { window.clearInterval(timer); timer = null; }
      }
      function start() {
        if (reduceMotion || timer || !visible) return;
        timer = window.setInterval(function () { go(index + 1); }, DELAY);
      }
      function restart() { stop(); start(); }

      if (prevBtn) prevBtn.addEventListener("click", function () { go(index - 1); restart(); });
      if (nextBtn) nextBtn.addEventListener("click", function () { go(index + 1); restart(); });
      for (var b = 0; b < dots.length; b++) {
        dots[b].addEventListener("click", function (event) {
          go(Number(event.currentTarget.getAttribute("data-carousel-dot")));
          restart();
        });
      }
      carousel.addEventListener("mouseenter", stop);
      carousel.addEventListener("mouseleave", start);
      carousel.addEventListener("focusin", stop);
      carousel.addEventListener("focusout", start);
      carousel.addEventListener("keydown", function (event) {
        if (event.key === "ArrowLeft") { go(index - 1); restart(); }
        else if (event.key === "ArrowRight") { go(index + 1); restart(); }
      });
      var touchX = null;
      carousel.addEventListener("touchstart", function (event) {
        touchX = event.changedTouches[0].clientX;
      }, { passive: true });
      carousel.addEventListener("touchend", function (event) {
        if (touchX === null) return;
        var dx = event.changedTouches[0].clientX - touchX;
        touchX = null;
        if (Math.abs(dx) > 40) { go(index + (dx < 0 ? 1 : -1)); restart(); }
      }, { passive: true });
      // 只在轮播进入视口时自动播放, 离开视口暂停
      if ("IntersectionObserver" in window) {
        var io = new IntersectionObserver(function (entries) {
          visible = entries[0].isIntersecting;
          if (visible) start(); else stop();
        }, { threshold: 0.2 });
        io.observe(carousel);
      } else {
        visible = true;
        start();
      }
      go(0);
    }
  });
})();
