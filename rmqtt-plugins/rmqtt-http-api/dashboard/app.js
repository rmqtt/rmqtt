/* ============================================================
   RMQTT Dashboard — main application
   Vue 3 app + hash router + global store + i18n + multi-level nav
   ============================================================ */
const { createApp } = Vue;

// Page component registry (title is resolved through $t(titleKey))
// Accepts {path, component, titleKey, isLogin} entries as well as function matchers
const pageRegistry = {
  '#/login':    { component: 'LoginPage',        titleKey: 'login.title',     isLogin: true },
  '#/':         { component: 'OverviewPage',     titleKey: 'nav.overview',    isLogin: false },
  '#/clients':  { component: 'ClientsPage',      titleKey: 'nav.clients',     isLogin: false },
  '#/clients/detail': { component: 'ClientDetailPage', titleKey: 'clients.detail_title', isLogin: false },
  '#/subscriptions': { component: 'SubscriptionsPage', titleKey: 'nav.subscriptions', isLogin: false },
  '#/retains':   { component: 'RetainsPage',      titleKey: 'nav.retained',    isLogin: false },
  '#/publish':  { component: 'PublishPage',      titleKey: 'nav.publish',     isLogin: false },
  '#/plugins':  { component: 'PluginsPage',      titleKey: 'nav.plugins',     isLogin: false },
};

const App = Vue.defineComponent({
  name: 'App',
  components: {
    AppLayout,
    SidebarNav: window.SidebarNav,
    MetricCard,
    LoginPage,
    OverviewPage,
    ClientsPage,
    ClientDetailPage: window.ClientDetailPage,
    SubscriptionsPage,
    RetainsPage: window.RetainsPage,
    PublishPage,
    PluginsPage,
  },
  data() {
    return {
      currentHash: location.hash || '#/login',
      pageCache: {},
      localeVersion: 0,
      localeState: { version: 0 },  // reactive object injected into components
      sidebarCollapsed: store.getSidebarCollapsed(),
      currentTheme: store.getTheme(),
      // Quick search
      showSearch: false,
      searchQuery: '',
      searchResults: [],
      searchLoading: false,
      searchTimer: null,
      // Language dropdown
      showLangDropdown: false,
      _langShowTimer: null,
      _langLeaveTimer: null,
    };
  },
  provide() {
    return { localeState: this.localeState };
  },
  computed: {
    currentPageSpec() {
      // Split the route at '?': '#/clients/detail?clientid=xxx' -> '#/clients/detail'
      const path = (this.currentHash || '').split('?')[0];
      return pageRegistry[path] || pageRegistry['#/'];
    },
    currentPage() {
      return this.currentPageSpec;
    },
    currentPageTitle() {
      void this.localeVersion;
      return this.$t(this.currentPageSpec.titleKey);
    },
    currentLang() {
      void this.localeVersion;
      return window.i18n.locale;
    },
    langItems() {
      return [
        { value: 'en',    label: 'English' },
        { value: 'zh-CN', label: '简体中文' },
        { value: 'zh-TW', label: '繁體中文' },
        { value: 'ru',    label: 'Русский' },
        { value: 'fr',    label: 'Français' },
        { value: 'es',    label: 'Español' },
        { value: 'de',    label: 'Deutsch' },
        { value: 'pt',    label: 'Português' },
        { value: 'it',    label: 'Italiano' },
        { value: 'hi',    label: 'हिन्दी' },
        { value: 'ar',    label: 'العربية' },
        { value: 'bn',    label: 'বাংলা' },
      ];
    },
    currentLangLabel() {
      void this.localeVersion;
      var items = this.langItems;
      for (var i = 0; i < items.length; i++) {
        if (items[i].value === window.i18n.locale) return items[i].label;
      }
      return window.i18n.locale;
    },
  },
  methods: {
    onHashChange() {
      const hash = location.hash || '#/login';
      this.currentHash = hash;

      if (hash !== '#/login' && !store.isLoggedIn()) {
        location.hash = '#/login';
        return;
      }
      if (hash === '#/login' && store.isLoggedIn()) {
        location.hash = '#/';
      }
    },
    navigate(hash) {
      location.hash = hash;
    },
    toggleSidebar() {
      this.sidebarCollapsed = !this.sidebarCollapsed;
      store.setSidebarCollapsed(this.sidebarCollapsed);
    },
    toggleTheme() {
      this.currentTheme = this.currentTheme === 'dark' ? 'light' : 'dark';
      store.setTheme(this.currentTheme);
    },
    // Quick search
    toggleSearch() {
      this.showSearch = !this.showSearch;
      if (this.showSearch) {
        this.searchQuery = '';
        this.searchResults = [];
        this.$nextTick(function() {
          var el = document.getElementById('quickSearchInput');
          if (el) el.focus();
        });
      }
    },
    onSearchInput() {
      var self = this;
      if (this.searchTimer) clearTimeout(this.searchTimer);
      var q = this.searchQuery.trim();
      if (q.length < 1) {
        this.searchResults = [];
        return;
      }
      this.searchTimer = setTimeout(function() {
        self.doSearch(q);
      }, 300);
    },
    async doSearch(query) {
      this.searchLoading = true;
      try {
        var data = await http.get('/clients', { clientid: query, limit: 10 });
        this.searchResults = Array.isArray(data) ? data : [];
      } catch (e) {
        this.searchResults = [];
      } finally {
        this.searchLoading = false;
      }
    },
    selectSearchResult(clientId) {
      this.showSearch = false;
      this.searchQuery = '';
      this.searchResults = [];
      // Go to the clients page, carrying clientid in the URL hash
      location.hash = '#/clients?clientid=' + encodeURIComponent(clientId);
    },
    handleSearchBlur() {
      var self = this;
      // Close with a delay so a click on a result item can land first
      setTimeout(function() { self.showSearch = false; }, 200);
    },
    async switchLang(locale) {
      await window.i18n.setLocale(locale);
      this.showLangDropdown = false;
    },
    toggleLangDropdown() {
      if (this._langShowTimer) clearTimeout(this._langShowTimer);
      this.showLangDropdown = !this.showLangDropdown;
    },
    onLangEnter() {
      if (this._langLeaveTimer) clearTimeout(this._langLeaveTimer);
      if (!this.showLangDropdown && !this._langShowTimer) {
        var self = this;
        this._langShowTimer = setTimeout(function() {
          self.showLangDropdown = true;
          self._langShowTimer = null;
        }, 350);
      }
    },
    onLangLeave() {
      var self = this;
      if (this._langShowTimer) {
        clearTimeout(this._langShowTimer);
        this._langShowTimer = null;
      }
      if (this._langLeaveTimer) clearTimeout(this._langLeaveTimer);
      this._langLeaveTimer = setTimeout(function() {
        self.showLangDropdown = false;
        self._langLeaveTimer = null;
      }, 350);
    },
    onLangDocClick(e) {
      var el = this.$el;
      if (el && !el.contains(e.target)) {
        this.showLangDropdown = false;
        document.removeEventListener('click', this.onLangDocClick);
      }
    },
    refresh() {
      location.reload();
    },
    logout() {
      store.clearToken();
      location.hash = '#/login';
    },
  },
  mounted() {
    this.onHashChange();
    window.addEventListener('hashchange', () => this.onHashChange());

    // On startup, verify whether an already stored token is still valid
    if (store.isLoggedIn()) {
      http.get('/brokers').catch(function() {
        store.clearToken();
        location.hash = '#/login';
      });
    }

    window.addEventListener('locale-changed', () => {
      this.localeVersion++;
      this.localeState.version++;
    });

    // Node count badge
    const updateNodes = async () => {
      try {
        var [healthData, nodesData] = await Promise.all([
          http.get('/health/check').catch(function() { return null; }),
          http.get('/nodes').catch(function() { return null; }),
        ]);
        var total = 0;
        var online = 0;
        var isRunning = false;
        if (healthData) {
          total = healthData.nodes ? healthData.nodes.length : 0;
          isRunning = healthData.running;
        }
        if (nodesData) {
          var list = Array.isArray(nodesData) ? nodesData : [];
          online = list.filter(function(n) { return n.running; }).length;
        }
        var allUp = online >= total && total > 0;
        var el = document.getElementById('nodeCount');
        if (el) {
          el.textContent = online + '/' + total;
          if (!isRunning) {
            el.className = 'node-badge critical';
          } else if (!allUp) {
            el.className = 'node-badge warning';
          } else {
            el.className = 'node-badge healthy';
          }
        }
      } catch (_) {
        var el = document.getElementById('nodeCount');
        if (el) {
          el.textContent = '-/-';
          el.className = 'node-badge critical';
        }
      }
    };
    updateNodes();
    setInterval(updateNodes, 10000);
  },
  template: `
    <div>
      <login-page v-if="currentHash === '#/login' && !store.isLoggedIn()" />

      <template v-else-if="store.isLoggedIn()">
        <app-layout :collapsed="sidebarCollapsed">
          <template #sidebar>
            <sidebar-nav
              :collapsed="sidebarCollapsed"
              :current-hash="currentHash"
              :locale-version="localeVersion"
              @navigate="navigate"
              @toggle-collapse="toggleSidebar" />
          </template>
          <template #topbar>
            <div class="topbar-left">
              <span class="page-title">{{ currentPageTitle }}</span>
            </div>
            <div class="topbar-right">
              <!-- Quick search -->
              <div class="quick-search" :class="{ active: showSearch }">
                <button class="btn-icon search-toggle disabled" title="Search (unavailable)" tabindex="-1">&#128269;</button>
                <div v-if="showSearch" class="search-overlay" @click.self="showSearch = false"></div>
                <div v-if="showSearch" class="search-panel">
                  <input id="quickSearchInput" class="search-input" type="text"
                         v-model="searchQuery" @input="onSearchInput" @blur="handleSearchBlur"
                         :placeholder="$t('nav.search_placeholder')" />
                  <div class="search-results" v-if="searchResults.length > 0">
                    <div v-for="c in searchResults" :key="c.clientid"
                         class="search-result-item" @mousedown.prevent="selectSearchResult(c.clientid)">
                      <span class="search-result-id">{{ c.clientid }}</span>
                      <span class="search-result-node">{{ c.node || '-' }}</span>
                      <span class="search-result-ip">{{ c.ipaddress || '-' }}</span>
                      <span :class="c.connected ? 'status-online' : 'status-offline'">
                        {{ c.connected ? '●' : '○' }}
                      </span>
                    </div>
                  </div>
                  <div class="search-empty" v-else-if="searchQuery.length >= 1 && !searchLoading">
                    {{ $t('common.no_results') }}
                  </div>
                </div>
              </div>
              <button class="btn-icon" :title="currentTheme === 'dark' ? $t('common.light_mode') : $t('common.dark_mode')" @click="toggleTheme">
                {{ currentTheme === 'dark' ? '&#9788;' : '&#9790;' }}
              </button>
              <div class="lang-select-wrapper"
                   @mouseenter="showLangDropdown = true"
                   @mouseleave="onLangLeave">
                <button class="lang-select-trigger" @click.stop="toggleLangDropdown()">
                  {{ currentLangLabel }}
                  <span class="lang-select-arrow">&#9660;</span>
                </button>
                <div v-if="showLangDropdown" class="lang-select-dropdown" @mouseenter="onLangEnter" @mouseleave="onLangLeave">
                  <button v-for="item in langItems" :key="item.value"
                          class="lang-select-item" :class="{ active: currentLang === item.value }"
                          @click="switchLang(item.value)">{{ item.label }}</button>
                </div>
              </div>
              <span class="node-badge" id="nodeCount">-</span>
              <button class="btn-icon" :title="$t('common.refresh')" @click="refresh">&#x27F3;</button>
              <button class="btn-icon" :title="$t('common.logout')" @click="logout">&#x23F0;</button>
            </div>
          </template>

          <component :is="currentPage.component" :key="localeVersion"></component>
        </app-layout>
      </template>
    </div>
  `,
});

;(async function() {
  // Initialise theme
  store.setTheme(store.getTheme());
  await window.i18n.init();
  const app = createApp(App);
  app.config.globalProperties.store = window.store;
  app.use(window.i18n);
  app.component('pagination', window.Pagination);
  app.component('metric-card', window.MetricCard);
  app.component('datetime-picker', window.DatetimePicker);
  app.mount('#app');
})();
