/* ============================================================
   RMQTT Dashboard — global store
   ============================================================ */
window.store = {
  getToken() {
    return localStorage.getItem('dashboard_token');
  },
  setToken(token) {
    localStorage.setItem('dashboard_token', token);
  },
  clearToken() {
    localStorage.removeItem('dashboard_token');
  },

  isLoggedIn() {
    return !!this.getToken();
  },

  /** Locale preference */
  getLocale() {
    return localStorage.getItem('dashboard_locale') || '';
  },
  setLocale(locale) {
    localStorage.setItem('dashboard_locale', locale);
  },

  /** Sidebar collapsed state */
  getSidebarCollapsed() {
    return localStorage.getItem('dashboard_sidebar_collapsed') === 'true';
  },
  setSidebarCollapsed(collapsed) {
    localStorage.setItem('dashboard_sidebar_collapsed', collapsed ? 'true' : 'false');
  },

  /** Theme: dark / light */
  getTheme() {
    return localStorage.getItem('dashboard_theme') || 'dark';
  },
  setTheme(theme) {
    localStorage.setItem('dashboard_theme', theme);
    document.documentElement.setAttribute('data-theme', theme);
  },
};
