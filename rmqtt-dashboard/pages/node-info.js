/* ============================================================
   RMQTT Dashboard — 节点详情页
   显示节点信息 + 节点统计（count/max）
   ============================================================ */
;(function() {
  'use strict';

  const { ref, onMounted } = Vue;

  function formatBoottime(str) {
  if (!str) return '-';
  return str.replace(/^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}).*$/, '$1');
}

window.NodeInfoPage = Vue.defineComponent({
    name: 'NodeInfoPage',
    template: `
      <div class="node-info-page">
        <!-- 顶部栏：返回 + 刷新 -->
        <div class="node-info-topbar">
          <a class="back-link" @click="goBack">&larr; 返回节点列表</a>
          <button class="btn-icon refresh-btn" @click="refresh" title="刷新">&#x21bb;</button>
        </div>

        <!-- 加载态 -->
        <div v-if="loading" class="loading-text">加载中...</div>

        <!-- 错误态 -->
        <div v-else-if="error" class="error-text">加载失败: {{ error }}</div>

        <template v-else>
          <div class="node-info-columns">
            <!-- ── 节点信息 ── -->
            <div class="info-section">
              <h3 class="section-title">节点信息</h3>
              <div class="info-grid">
                <div class="info-row">
                  <span class="info-label">节点名称</span>
                  <span class="info-value">{{ node.node_name || '-' }}</span>
                </div>
                <div class="info-row">
                  <span class="info-label">状态</span>
                  <span class="info-value">
                    <span :class="node.running ? 'status-online' : 'status-offline'">
                      {{ node.running ? '● 在线' : '○ 离线' }}
                    </span>
                  </span>
                </div>
                <div class="info-row">
                  <span class="info-label">版本</span>
                  <span class="info-value">{{ node.version || '-' }}</span>
                </div>
                <div class="info-row">
                  <span class="info-label">Rust 版本</span>
                  <span class="info-value">{{ node.rustc_version || '-' }}</span>
                </div>
              <div class="info-row">
                <span class="info-label">操作系统 CPU 负载</span>
                <span class="info-value">{{ formatCpuLoad(node.load1, node.load5, node.load15) }}</span>
              </div>
              <div class="info-row">
                <span class="info-label">操作系统内存</span>
                  <span class="info-value">{{ formatBytes(node.memory_used) }} / {{ formatBytes(node.memory_total) }}</span>
                </div>
                <div class="info-row">
                  <span class="info-label">磁盘</span>
                  <span class="info-value">{{ formatBytes(node.disk_free) }} / {{ formatBytes(node.disk_total) }}</span>
                </div>
                <div class="info-row">
                  <span class="info-label">运行时长</span>
                  <span class="info-value">{{ formatUptime(node.uptime) || '-' }}</span>
                </div>
                <div class="info-row">
                  <span class="info-label">启动时间</span>
                  <span class="info-value">{{ formatBoottime(node.boottime) || '-' }}</span>
                </div>
              </div>
            </div>

            <!-- ── 节点统计 ── -->
            <div class="info-section">
              <h3 class="section-title">节点统计</h3>
              <div class="stats-grid">
                <div v-for="s in statsItems" :key="s.key" class="stats-row">
                  <span class="stats-label">{{ s.label }}</span>
                  <span class="stats-value">{{ s.count }} / {{ s.max }}</span>
                </div>
              </div>
            </div>

            <!-- ── 功能支持 ── -->
            <div class="info-section">
              <h3 class="section-title">{{ $t('node_detail.features_title') }}</h3>
              <div v-if="featuresLoading" class="loading-text">{{ $t('common.loading') }}...</div>
              <div v-else-if="featuresError" class="error-text">{{ featuresError }}</div>
              <div v-else-if="features" class="feature-grid">
                <div v-for="f in featureItems" :key="f.key" class="feature-row">
                  <span class="feature-label">{{ f.label }}</span>
                  <span class="feature-badge" :class="f.enabled ? 'badge-on' : 'badge-off'">
                    {{ f.enabled ? $t('common.enabled') : $t('common.disabled') }}
                  </span>
                </div>
              </div>
            </div>
          </div>
        </template>
      </div>
    `,
    setup() {
      const localeState = Vue.inject('localeState');
      function $t(key, params) {
        void localeState.version;
        return window.i18n.$t(key, params);
      }

      const node = ref(null);
      const stats = ref({});
      const loading = ref(true);
      const error = ref(null);
      const features = ref(null);
      const featuresLoading = ref(false);
      const featuresError = ref(null);

      const FEATURES = [
        { key: 'retain',                labelKey: 'overview.features_retain' },
        { key: 'message_storage',       labelKey: 'overview.features_message_storage' },
        { key: 'session_storage',       labelKey: 'overview.features_session_storage' },
        { key: 'delayed',               labelKey: 'overview.features_delayed' },
        { key: 'shared_subscription',   labelKey: 'overview.features_shared_subscription' },
        { key: 'auto_subscription',     labelKey: 'overview.features_auto_subscription' },
      ];

      const featureItems = Vue.computed(function() {
        void localeState.version;
        var f = features.value;
        if (!f) return [];
        return FEATURES.map(function(item) {
          return {
            key: item.key,
            label: $t(item.labelKey),
            enabled: !!f[item.key],
          };
        });
      });

      const statsItems = [
        { key: 'connections',      label: '连接数' },
        { key: 'sessions',         label: '会话' },
        { key: 'topics',           label: '主题数' },
        { key: 'subscriptions',    label: '订阅' },
        { key: 'retaineds',        label: '保留消息' },
        { key: 'subscriptions_shared', label: '共享订阅' },
      ];

      function getNodeIdFromHash() {
        var hash = location.hash;
        var m = hash.match(/^#\/node-info\/(\d+)/);
        return m ? m[1] : null;
      }

      function formatCpuLoad(load1, load5, load15) {
        if (load1 == null) return '-';
        return load1.toFixed(2) + '/' + load5.toFixed(2) + '/' + load15.toFixed(2);
      }

      function formatBytes(bytes) {
        if (!bytes) return '-';
        var gb = bytes / (1024 * 1024 * 1024);
        return gb >= 1 ? gb.toFixed(1) + 'G' : (bytes / (1024 * 1024)).toFixed(0) + 'M';
      }

      function formatUptime(str) {
        if (!str) return '-';
        var isZh = window.i18n && window.i18n.locale && window.i18n.locale.indexOf('zh') === 0;
        var parts = [];
        str.replace(/(\d+)\s*(days?|hours?|minutes?|seconds?)/gi, function(m, num, unit) {
          var key = unit.toLowerCase().replace(/s$/, '');
          parts.push({ key: key, num: parseInt(num, 10) });
        });
        if (parts.length === 0) return str;
        var startIdx = parts.findIndex(function(p) { return p.num > 0; });
        if (startIdx === -1) startIdx = parts.length - 1;
        var units = isZh
          ? { day: '天', hour: '小时', minute: '分', second: '秒' }
          : { day: ' day ', hour: ' hour ', minute: ' minute ', second: ' second ' };
        var result = '';
        for (var i = startIdx; i < parts.length; i++) {
          var p = parts[i];
          var u = units[p.key] || ' ' + p.key + ' ';
          result += p.num + u;
        }
        return result.trim() || '-';
      }

      function getStatValue(key) {
        var count = stats.value[key + '.count'];
        var max = stats.value[key + '.max'];
        return {
          count: count != null ? count : '-',
          max: max != null ? max : '-',
        };
      }

      async function fetchData() {
        var nodeId = getNodeIdFromHash();
        if (!nodeId) {
          error.value = '无效的节点 ID';
          loading.value = false;
          return;
        }

        loading.value = true;
        error.value = null;
        featuresLoading.value = true;
        featuresError.value = null;

        try {
          var [nodeRes, statsRes, featuresRes] = await Promise.all([
            http.get('/nodes/' + nodeId).catch(function() { return null; }),
            http.get('/stats/' + nodeId).catch(function() { return null; }),
            http.get('/features/' + nodeId).catch(function() { return null; }),
          ]);

          if (nodeRes) {
            node.value = nodeRes;
          } else {
            error.value = '节点信息获取失败';
            loading.value = false;
            return;
          }

          if (statsRes && statsRes.stats) {
            stats.value = statsRes.stats;
          }

          if (featuresRes && typeof featuresRes === 'object' && featuresRes.features) {
            features.value = featuresRes.features;
          } else {
            featuresError.value = '功能支持信息获取失败';
          }

          loading.value = false;
          featuresLoading.value = false;
        } catch (e) {
          error.value = e.message || '未知错误';
          loading.value = false;
          featuresLoading.value = false;
        }
      }

      function goBack() {
        location.hash = '#/';
      }

      function refresh() {
        fetchData();
      }

      // statsItems 加上实际数据
      const statsItemsWithValues = Vue.computed(function() {
        void localeState.version;
        var s = stats.value;
        return statsItems.map(function(item) {
          var v = getStatValue(item.key);
          return {
            key: item.key,
            label: item.label,
            count: v.count,
            max: v.max,
          };
        });
      });

      onMounted(function() {
        fetchData();
      });

      return {
        node,
        stats,
        loading,
        error,
        features,
        featuresLoading,
        featuresError,
        featureItems,
        statsItems: statsItemsWithValues,
        formatCpuLoad,
        formatBytes,
        formatUptime,
        formatBoottime,
        goBack,
        refresh,
      };
    },
  });
})();
