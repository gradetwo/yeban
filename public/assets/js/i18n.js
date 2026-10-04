/* 双语切换 (中文 / English)
 *
 * 设计要点:
 *   1. **渐进增强**: HTML 里已经写好了中文原文。即使 JSON 取不到、JS 被禁用,
 *      页面依然完整可读 —— 只是不切换语言。这是刻意的, 因为静态站点的第一要求是"能看"。
 *   2. **单一契约**: 所有可翻译文本用 data-i18n / data-i18n-attr 标注;
 *      `scripts/check-site.mjs` 会断言两份词典键集完全相同、且 HTML 用到的键都存在。
 *   3. **语言持久化**: localStorage + URL 查询参数 (?lang=en)。URL 优先,
 *      这样别人分享的链接会打开对应语言 (并且与 <link rel="alternate" hreflang> 一致)。
 *   4. **无障碍**: 切换后同步 <html lang>, 让读屏软件换用正确的语音。
 */
(function () {
  "use strict";

  var STORAGE_KEY = "yeban-lang";
  var SUPPORTED = ["zh", "en"];
  var HTML_LANG = { zh: "zh-Hans", en: "en" };
  var dict = null;
  var current = "zh";

  function fromUrl() {
    try {
      var v = new URLSearchParams(window.location.search).get("lang");
      return SUPPORTED.indexOf(v) >= 0 ? v : null;
    } catch (e) {
      return null;
    }
  }

  function fromStore() {
    try {
      var v = localStorage.getItem(STORAGE_KEY);
      return SUPPORTED.indexOf(v) >= 0 ? v : null;
    } catch (e) {
      return null;
    }
  }

  function fromBrowser() {
    var langs = navigator.languages || [navigator.language || "zh"];
    for (var i = 0; i < langs.length; i++) {
      var tag = String(langs[i]).toLowerCase();
      if (tag.indexOf("zh") === 0) return "zh";
      if (tag.indexOf("en") === 0) return "en";
    }
    return "zh";
  }

  function t(key) {
    if (!dict) return null;
    return Object.prototype.hasOwnProperty.call(dict, key) ? dict[key] : null;
  }

  function applyToDom() {
    if (!dict) return;

    var nodes = document.querySelectorAll("[data-i18n]");
    for (var i = 0; i < nodes.length; i++) {
      var value = t(nodes[i].getAttribute("data-i18n"));
      if (value !== null) nodes[i].textContent = value;
    }

    // data-i18n-attr="alt:a11y.logo,aria-label:nav.label"
    var attrNodes = document.querySelectorAll("[data-i18n-attr]");
    for (var j = 0; j < attrNodes.length; j++) {
      var pairs = attrNodes[j].getAttribute("data-i18n-attr").split(",");
      for (var k = 0; k < pairs.length; k++) {
        var parts = pairs[k].split(":");
        if (parts.length !== 2) continue;
        var attr = parts[0].trim();
        var val = t(parts[1].trim());
        if (val !== null && attr) attrNodes[j].setAttribute(attr, val);
      }
    }

    var langNodes = document.querySelectorAll("[data-i18n-lang]");
    for (var m = 0; m < langNodes.length; m++) {
      var lv = t(langNodes[m].getAttribute("data-i18n-lang"));
      if (lv !== null) langNodes[m].setAttribute("lang", lv);
    }
  }

  function syncButtons() {
    var buttons = document.querySelectorAll("[data-lang-set]");
    for (var i = 0; i < buttons.length; i++) {
      buttons[i].setAttribute("aria-pressed", String(buttons[i].getAttribute("data-lang-set") === current));
    }
  }

  function apply(lang, persist) {
    current = SUPPORTED.indexOf(lang) >= 0 ? lang : "zh";
    document.documentElement.setAttribute("lang", HTML_LANG[current]);
    if (persist) {
      try { localStorage.setItem(STORAGE_KEY, current); } catch (e) { /* 忽略 */ }
    }
    if (dict) {
      applyToDom();
      syncButtons();
    }
  }

  function fetchDict(lang) {
    return fetch("/assets/i18n/" + lang + ".json", { cache: "force-cache" })
      .then(function (res) {
        if (!res.ok) throw new Error("i18n " + lang + " HTTP " + res.status);
        return res.json();
      })
      .then(function (json) { return json; });
  }

  function init() {
    var initial = fromUrl() || fromStore() || fromBrowser();
    current = initial;

    // 先取当前语言; 另一份词典在首次切换时再取 (省一次请求)
    fetchDict(initial)
      .then(function (json) {
        dict = json;
        apply(initial, false);
        // 其余语言预取: 切换要做到无感, 而不是点一下等一个请求
        SUPPORTED.forEach(function (lang) {
          if (lang === initial || lang === "zh") return;
          fetchDict(lang).then(function (json2) {
            if (lang !== current) return;
            dict = json2;
            applyToDom();
          }).catch(function () { /* 预取失败不影响主流程 */ });
        });
      })
      .catch(function () {
        // 词典取不到: 保持 HTML 里的默认语言, 页面依然完整可读
        apply(initial, false);
      });

    // zh 也需要能主动切换 (而不是只在初始语言是 zh 时)
    SUPPORTED.forEach(function (lang) {
      if (lang === "zh") return;
      fetchDict(lang).catch(function () { /* 预取失败忽略 */ });
    });
  }

  window.YebanI18n = {
    set: function (lang) {
      var target = SUPPORTED.indexOf(lang) >= 0 ? lang : "zh";
      // 已经加载过就直接用; 否则先取
      var need = !dict || current !== target;
      apply(target, true);
      fetchDict(target).then(function (json) {
        if (target !== current) return;
        dict = json;
        applyToDom();
        syncButtons();
      }).catch(function () { /* 保持原文 */ });
      return need;
    },
    // 切换语言后, 把 URL 里的 lang 参数同步一下, 便于复制分享
    syncUrl: function (lang) {
      try {
        var url = new URL(window.location.href);
        if (lang === "zh") url.searchParams.delete("lang");
        else url.searchParams.set("lang", lang);
        window.history.replaceState(null, "", url.toString());
      } catch (e) { /* 忽略 */ }
    }
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
