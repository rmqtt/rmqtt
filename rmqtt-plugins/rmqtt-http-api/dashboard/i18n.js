/* ============================================================
   RMQTT Dashboard — i18n module
   Hand-rolled Vue 3 plugin: asynchronously loads JSON locale bundles, zero external deps
   Usage: window.i18n.$t('nav.overview')     -> "Overview"
              window.i18n.$t('clients.disconnect_confirm', {clientId: 'abc'}) -> "Kick client abc?"
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
      this._localeVer = 13;  // locale file version, bump it after editing to bypass the browser cache
    }

    /** Init: detect the language, then preload every locale bundle */
    async init() {
      var self = this;
      const saved = window.store?.getLocale();
      // Prefer the user's saved choice; default to English on first visit
      this.locale = saved
        ? (LOCALE_MAP[saved.toLowerCase()] || FALLBACK)
        : FALLBACK;
      // Preload all locale bundles so switching needs no further HTTP request
      var locales = ['zh-CN', 'zh-TW', 'en', 'ru', 'fr', 'es', 'de', 'pt', 'it', 'hi', 'ar', 'bn'];
      await Promise.all(locales.map(function(l) { return self._load(l); }));
      // Make sure _messages belongs to the detected locale
      self._messages = self._cache[self.locale] || self._cache[FALLBACK] || {};
    }

    /** Load a JSON locale bundle asynchronously (cached in _cache) */
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

    /** Translate: supports dotted paths and parameter substitution */
    $t(key, params) {
      const val = key.split('.').reduce(function(o, k) { return o ? o[k] : undefined; }, this._messages);
      if (val == null) return key;
      if (!params) return val;
      return Object.entries(params).reduce(function(s, arr) {
        return s.replace(new RegExp('\\{' + arr[0] + '\\}', 'g'), arr[1]);
      }, val);
    }

    /** Switch locale (read straight from the cache, no HTTP request) */
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

    /** Vue 3 plugin install: expose $t on the global properties */
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
