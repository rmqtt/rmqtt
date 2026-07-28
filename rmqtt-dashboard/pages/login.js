/* ============================================================
   RMQTT Dashboard — 登录页（国际化）
   用户输入 http-api 的 Bearer Token
   ============================================================ */
window.LoginPage = Vue.defineComponent({
  name: 'LoginPage',
  template: `
    <div class="login-page">
      <div class="login-card">
        <h1>RMQTT</h1>
        <p>{{ $t('login.subtitle') }}</p>
        <div class="form-group">
          <label>{{ $t('login.title') }}</label>
          <div class="password-wrapper">
            <input class="form-input" :type="showToken ? 'text' : 'password'" v-model="token"
                   :placeholder="$t('login.token_placeholder')" @keyup.enter="login" />
            <button class="password-toggle" type="button" @click="showToken = !showToken"
                    :title="showToken ? $t('login.hide_token') : $t('login.show_token')">
              <span v-text="showToken ? '🙈' : '👁'"></span>
            </button>
          </div>
        </div>
        <div class="form-group" v-if="error" style="color:var(--red);font-size:13px;">
          {{ error }}
        </div>
        <button class="btn btn-primary" @click="login" :disabled="loading">
          {{ loading ? $t('login.verifying') : $t('login.submit') }}
        </button>
      </div>
    </div>
  `,
  setup() {
    const token = Vue.ref('');
    const showToken = Vue.ref(false);
    const loading = Vue.ref(false);
    const error = Vue.ref('');

    // $t 在 setup 中不可用，用 window.i18n.$t
    function $t(key, params) { return window.i18n.$t(key, params); }

    async function login() {
      if (!token.value.trim()) { error.value = $t('login.error_empty'); return; }
      loading.value = true;
      error.value = '';
      try {
        // 先保存 Token，再验证——确保验证请求携带 Authorization 头
        store.setToken(token.value.trim());
        const result = await http.get('/brokers');
        if (result) {
          location.hash = '#/';
        } else {
          // result 为 null 说明 http.js 捕获了 401 并清除了 Token
          throw new Error('Unauthorized');
        }
      } catch (e) {
        // 验证失败，清除无效 Token
        store.clearToken();
        error.value = $t('login.error_invalid');
      } finally {
        loading.value = false;
      }
    }

    return { token, showToken, loading, error, login };
  },
});
