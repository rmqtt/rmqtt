/* ============================================================
   RMQTT Dashboard — 插件管理页
   ============================================================ */
window.PluginsPage = Vue.defineComponent({
  name: 'PluginsPage',
  template: `
    <div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>插件名称</th>
              <th>版本</th>
              <th>状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="p in plugins" :key="p.name || p.Name">
              <td>{{ p.name || p.Name }}</td>
              <td>{{ p.version || p.Version || '-' }}</td>
              <td>
                <span :class="p.running || p.Running ? 'status-online' : 'status-offline'">
                  {{ p.running || p.Running ? '● 运行中' : '○ 已停止' }}
                </span>
              </td>
              <td>
                <button class="btn-icon" style="width:auto;padding:4px 12px;font-size:12px;"
                        @click="viewConfig(p)" :title="$t('plugins.config')">
                  {{ $t('plugins.config') }}
                </button>
              </td>
            </tr>
            <tr v-if="plugins.length === 0">
              <td colspan="4" style="text-align:center;color:var(--text-muted);padding:40px;">暂无数据</td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 配置弹窗 -->
      <div v-if="showConfig" style="position:fixed;inset:0;background:rgba(0,0,0,.6);display:flex;align-items:center;justify-content:center;z-index:100;">
        <div style="background:var(--bg-card);border:1px solid var(--border);border-radius:var(--radius);padding:24px;width:600px;max-height:80vh;overflow-y:auto;">
          <h3 style="margin-bottom:12px;">{{ configPluginName }} 配置</h3>
          <pre style="background:var(--bg);padding:12px;border-radius:var(--radius);font-size:12px;overflow-x:auto;white-space:pre-wrap;">{{ configContent }}</pre>
          <div style="margin-top:12px;text-align:right;">
            <button class="btn btn-primary" @click="showConfig = false">关闭</button>
          </div>
        </div>
      </div>
    </div>
  `,
  setup() {
    const plugins = Vue.ref([]);
    const showConfig = Vue.ref(false);
    const configPluginName = Vue.ref('');
    const configContent = Vue.ref('');

    async function loadPlugins() {
      try {
        const data = await http.get('/plugins');
        plugins.value = Array.isArray(data) ? data : [];
      } catch (e) {
        console.error(e);
      }
    }

    async function viewConfig(p) {
      const name = p.name || p.Name;
      configPluginName.value = name;
      showConfig.value = true;
      configContent.value = '加载中...';
      try {
        const data = await http.get('/plugins/' + encodeURIComponent(name) + '/config');
        configContent.value = JSON.stringify(data, null, 2);
      } catch (e) {
        configContent.value = '获取失败: ' + e.message;
      }
    }

    Vue.onMounted(loadPlugins);

    return { plugins, showConfig, configPluginName, configContent, viewConfig };
  },
});
