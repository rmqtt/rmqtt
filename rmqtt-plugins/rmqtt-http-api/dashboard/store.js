/* ============================================================
   RMQTT Dashboard — 全局状态管理
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

  /** 语言偏好 */
  getLocale() {
    return localStorage.getItem('dashboard_locale') || '';
  },
  setLocale(locale) {
    localStorage.setItem('dashboard_locale', locale);
  },

  /** 侧栏收缩状态 */
  getSidebarCollapsed() {
    return localStorage.getItem('dashboard_sidebar_collapsed') === 'true';
  },
  setSidebarCollapsed(collapsed) {
    localStorage.setItem('dashboard_sidebar_collapsed', collapsed ? 'true' : 'false');
  },

  /** 主题: dark / light */
  getTheme() {
    return localStorage.getItem('dashboard_theme') || 'dark';
  },
  setTheme(theme) {
    localStorage.setItem('dashboard_theme', theme);
    document.documentElement.setAttribute('data-theme', theme);
  },
};
