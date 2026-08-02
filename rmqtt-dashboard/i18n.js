/* ============================================================
   RMQTT Dashboard — 国际化 i18n 模块
   自研 Vue 3 插件，异步加载 JSON 语言包，零外部依赖
   用法：window.i18n.$t('nav.overview')     → "概览"
         window.i18n.$t('clients.disconnect_confirm', {clientId: 'abc'}) → "确认踢出客户端 abc ？"
   ============================================================ */
;(function() {
  'use strict';

  const LOCALE_MAP = {
    'zh-cn': 'zh-CN', 'zh-hans': 'zh-CN', 'zh-sg': 'zh-CN', 'zh': 'zh-CN',
    'zh-tw': 'zh-TW', 'zh-hk': 'zh-TW', 'zh-mo': 'zh-TW', 'zh-hant': 'zh-TW',
    'en': 'en', 'en-us': 'en', 'en-gb': 'en', 'en-au': 'en',
    'ru': 'ru', 'ru-ru': 'ru',
    'fr': 'fr', 'fr-fr': 'fr', 'fr-ca': 'fr', 'fr-ch': 'fr', 'fr-be': 'fr',
    'es': 'es', 'es-es': 'es', 'es-mx': 'es', 'es-ar': 'es',
    'de': 'de', 'de-de': 'de', 'de-at': 'de', 'de-ch': 'de',
    'pt': 'pt', 'pt-pt': 'pt', 'pt-br': 'pt',
    'it': 'it', 'it-it': 'it', 'it-ch': 'it',
    'hi': 'hi', 'hi-in': 'hi',
    'ar': 'ar', 'ar-sa': 'ar', 'ar-eg': 'ar', 'ar-ae': 'ar',
    'bn': 'bn', 'bn-bd': 'bn', 'bn-in': 'bn',
  };
  const FALLBACK = 'en';

  class I18n {
    constructor() {
      this.locale = FALLBACK;
      this._messages = {};
      this._cache = {};
      this._localeVer = 7;  // 语言文件版本号，修改后递增以绕过浏览器缓存
    }

    /** 初始化：检测语言 → 预加载全部语言包 */
    async init() {
      var self = this;
      const saved = window.store?.getLocale();
      // 优先使用用户之前保存的偏好；首次访问默认为英文
      this.locale = saved
        ? (LOCALE_MAP[saved.toLowerCase()] || FALLBACK)
        : FALLBACK;
      // 预加载所有语言包，切换时无需再次 HTTP 请求
      var locales = ['zh-CN', 'zh-TW', 'en', 'ru', 'fr', 'es', 'de', 'pt', 'it', 'hi', 'ar', 'bn'];
      await Promise.all(locales.map(function(l) { return self._load(l); }));
      // 确保 _messages 为检测到的语言
      self._messages = self._cache[self.locale] || self._cache[FALLBACK] || {};
    }

    /** 异步加载 JSON 语言包（缓存到 _cache） */
    async _load(locale) {
      if (this._cache[locale]) {
        this._messages = this._cache[locale];
        return;
      }
      try {
        const resp = await fetch('./locales/' + locale + '.json?_v=' + this._localeVer);
        if (!resp.ok) throw new Error('HTTP ' + resp.status);
        this._cache[locale] = await resp.json();
        this._messages = this._cache[locale];
      } catch (e) {
        console.warn('[i18n] Failed to load ' + locale + ', fallback to ' + FALLBACK + ':', e);
        if (locale === FALLBACK) {
          this._messages = {};
          return;
        }
        this.locale = FALLBACK;
        await this._load(FALLBACK);
      }
    }

    /** 翻译方法：支持点号路径 + 参数替换 */
    $t(key, params) {
      const val = key.split('.').reduce(function(o, k) { return o ? o[k] : undefined; }, this._messages);
      if (val == null) return key;
      if (!params) return val;
      return Object.entries(params).reduce(function(s, arr) {
        return s.replace(new RegExp('\\{' + arr[0] + '\\}', 'g'), arr[1]);
      }, val);
    }

    /** 切换语言（直接从缓存读取，无需 HTTP 请求） */
    async setLocale(locale) {
      const norm = LOCALE_MAP[locale.toLowerCase()];
      if (!norm || norm === this.locale) return;
      if (this._cache[norm]) {
        this._messages = this._cache[norm];
      } else {
        await this._load(norm);
      }
      this.locale = norm;
      if (window.store) window.store.setLocale(norm);
      window.dispatchEvent(new CustomEvent('locale-changed'));
    }

    /** Vue 3 插件安装：将 $t 注入全局属性 */
    install(app) {
      const self = this;
      app.config.globalProperties.$t = function(key, params) {
        return self.$t(key, params);
      };
      app.provide('i18n', self);
    }
  }

  window.i18n = new I18n();
})();
