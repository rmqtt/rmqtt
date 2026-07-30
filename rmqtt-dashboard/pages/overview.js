/* ============================================================
   RMQTT Dashboard — 概览页
   三标签布局：集群概览 / 节点 / 指标
   ============================================================ */
;(function() {
  'use strict';

  const { ref, onMounted, onUnmounted, nextTick } = Vue;

  window.OverviewPage = Vue.defineComponent({
    name: 'OverviewPage',
    template: `
      <div>
        <!-- 标签栏 -->
        <div class="tab-bar">
          <button v-for="tab in tabs" :key="tab.key"
                  class="tab-btn" :class="{ active: activeTab === tab.key }"
                  @click="activeTab = tab.key">{{ tab.label }}</button>
        </div>

        <!-- ─── Tab 1: 集群概览 ─── -->
        <div v-show="activeTab === 'overview'">
          <div class="overview-top">
            <div class="gauge-card" id="gaugeConnContainer"></div>
            <div class="msg-rate-card" id="msgRateContainer"></div>
            <div class="overview-metrics">
              <metric-card icon="💬" :label="$t('overview.total_sessions')" :value="stats.sessions" color="#22c55e"></metric-card>
              <metric-card icon="✓" :label="$t('overview.online_sessions')" :value="stats.connections" color="#3b82f6"></metric-card>
              <metric-card icon="#" :label="$t('overview.topics_count')" :value="stats.topics" color="#8b5cf6"></metric-card>
              <metric-card icon="+" :label="$t('overview.subscriptions_count')" :value="stats.subscriptions" color="#f59e0b"></metric-card>
              <metric-card icon="↗" :label="$t('overview.shared_subscriptions_count')" :value="stats.sharedSubscriptions" color="#06b6d4"></metric-card>
              <metric-card icon="📋" :label="$t('overview.retained_count')" :value="stats.retained" color="#22c55e"></metric-card>
            </div>
          </div>

          <!-- 节点信息卡片 -->
          <div class="node-info-card">
            <div class="node-info-header">
              <div class="node-info-title">
                <span class="node-name">{{ nodes.length }} {{ $t('overview.nodes_count') }}</span>
              </div>
              <h3>{{ $t('overview.node_info') }}</h3>
              <a class="node-info-link" @click="goToNodeList">{{ $t('overview.view_node_list') }}</a>
            </div>
            <div v-for="n in nodes" :key="n.node_id" class="node-info-body">
              <div class="node-info-icon">
                <svg viewBox="0 0 100 100" class="hexagon-icon">
                  <defs>
                    <linearGradient id="hexGrad" x1="0%" y1="0%" x2="100%" y2="100%">
                      <stop offset="0%" style="stop-color:#3b82f6;stop-opacity:1" />
                      <stop offset="100%" style="stop-color:#8b5cf6;stop-opacity:1" />
                    </linearGradient>
                    <linearGradient id="hexGradOffline" x1="0%" y1="0%" x2="100%" y2="100%">
                      <stop offset="0%" style="stop-color:#ef4444;stop-opacity:1" />
                      <stop offset="100%" style="stop-color:#dc2626;stop-opacity:1" />
                    </linearGradient>
                  </defs>
                  <polygon points="50,5 95,27.5 95,72.5 50,95 5,72.5 5,27.5" :fill="n.running ? 'url(#hexGrad)' : 'url(#hexGradOffline)'" />
                </svg>
              </div>
              <div class="node-info-grid">
                <div class="node-info-item">
                  <span class="node-info-label">{{ $t('overview.node_name') }}</span>
                  <span class="node-info-value">{{ n.node_name || '-' }}</span>
                </div>
                <div class="node-info-item">
                  <span class="node-info-label">{{ $t('overview.node_uptime') }}</span>
                  <span class="node-info-value">{{ formatUptime(n.uptime) }}</span>
                </div>
                <div class="node-info-item">
                  <span class="node-info-label">{{ $t('overview.node_version') }}</span>
                  <span class="node-info-value">{{ n.version || '-' }}</span>
                </div>
                <div class="node-info-item">
                  <span class="node-info-label">{{ $t('overview.connections_count') }}</span>
                  <span class="node-info-value">{{ n.connections != null ? n.connections : '-' }}</span>
                </div>
                <div class="node-info-item">
                  <span class="node-info-label">{{ $t('overview.cpu_load') }}</span>
                  <span class="node-info-value">{{ n.load1 != null ? (n.load1.toFixed(2) + '/' + n.load5.toFixed(2) + '/' + n.load15.toFixed(2)) : '-' }}</span>
                </div>
                <div class="node-info-item">
                  <span class="node-info-label">{{ $t('overview.node_memory') }}</span>
                  <span class="node-info-value">{{ n.memory_used ? (formatBytes(n.memory_used) + '/' + formatBytes(n.memory_total)) : '-' }}</span>
                </div>
              </div>
            </div>
          </div>

          <div class="chart-toolbar">
            <span class="chart-toolbar-label">{{ $t('overview.time_range') }}</span>
            <button v-for="r in timeRanges" :key="r.key"
                    class="time-btn" :class="{ active: timeRange === r.key }"
                    @click="setTimeRange(r.key)">{{ r.label }}</button>
          </div>
          <div class="chart-grid">
            <div class="chart-card">
              <h3>{{ $t('overview.msg_in_trend') }}</h3>
              <div class="chart-box" id="chartMsgIn"></div>
            </div>
            <div class="chart-card">
              <h3>{{ $t('overview.msg_out_trend') }}</h3>
              <div class="chart-box" id="chartMsgOut"></div>
            </div>
            <div class="chart-card">
              <h3>{{ $t('overview.msg_dropped_trend') }}</h3>
              <div class="chart-box" id="chartMsgDropped"></div>
            </div>
            <div class="chart-card">
              <h3>{{ $t('overview.connections_trend') }}</h3>
              <div class="chart-box" id="chartConnections"></div>
            </div>
            <div class="chart-card">
              <h3>{{ $t('overview.topics_trend') }}</h3>
              <div class="chart-box" id="chartTopics"></div>
            </div>
            <div class="chart-card">
              <h3>{{ $t('overview.subscriptions_trend') }}</h3>
              <div class="chart-box" id="chartSubscriptions"></div>
            </div>
          </div>
        </div>

        <!-- ─── Tab 2: 节点 ─── -->
        <div v-show="activeTab === 'nodes'">
          <!-- ── 节点列表 ── -->
          <template v-if="!selectedNodeId">
            <h3 class="section-title">{{ $t('overview.nodes') }}</h3>
            <div class="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>{{ $t('overview.node_name_th') }}</th>
                    <th>{{ $t('overview.node_version') }}</th>
                    <th>{{ $t('overview.status') }}</th>
                    <th>{{ $t('overview.connections_count') }}</th>
                    <th>{{ $t('overview.os_cpu_load') }}</th>
                    <th>{{ $t('overview.os_memory') }}</th>
                    <th>{{ $t('overview.uptime') }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="n in nodes" :key="n.node_id" class="clickable-row" @click="showNodeDetail(n.node_id)">
                    <td>{{ n.node_name }}</td>
                    <td>{{ n.version || '-' }}</td>
                    <td><span :class="n.running ? 'status-online' : 'status-offline'">
                      {{ n.running ? $t('overview.online') : $t('overview.offline') }}
                    </span></td>
                    <td>{{ n.connections }}</td>
                    <td>{{ formatCpuLoad(n.load1, n.load5, n.load15) }}</td>
                    <td>{{ formatBytes(n.memory_used) }}/{{ formatBytes(n.memory_total) }}</td>
                    <td>{{ formatUptime(n.uptime) }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </template>
          <!-- ── 节点详情 ── -->
          <template v-else>
            <div class="node-detail-back">
              <a class="back-link" @click="hideNodeDetail">&larr; 返回节点列表</a>
              <button class="btn-icon refresh-btn" @click="refreshNodeDetail" title="刷新">&#x21bb;</button>
            </div>
            <div v-if="nodeDetailLoading" class="loading-text">加载中...</div>
            <div v-else-if="nodeDetailError" class="error-text">加载失败: {{ nodeDetailError }}</div>
            <template v-else>
              <div class="node-info-columns">
                <div class="info-section">
                  <h3 class="section-title">节点信息</h3>
                  <div class="info-grid">
                    <div class="info-row"><span class="info-label">节点名称</span><span class="info-value">{{ nodeDetail.node_name || '-' }}</span></div>
                    <div class="info-row"><span class="info-label">状态</span><span class="info-value"><span :class="nodeDetail.running ? 'status-online' : 'status-offline'">{{ nodeDetail.running ? '● 在线' : '○ 离线' }}</span></span></div>
                    <div class="info-row"><span class="info-label">版本</span><span class="info-value">{{ nodeDetail.version || '-' }}</span></div>
                    <div class="info-row"><span class="info-label">Rust 版本</span><span class="info-value">{{ nodeDetail.rustc_version || '-' }}</span></div>
                    <div class="info-row"><span class="info-label">操作系统 CPU 负载</span><span class="info-value">{{ formatCpuLoad(nodeDetail.load1, nodeDetail.load5, nodeDetail.load15) }}</span></div>
                    <div class="info-row"><span class="info-label">操作系统内存</span><span class="info-value">{{ formatBytes(nodeDetail.memory_used) }} / {{ formatBytes(nodeDetail.memory_total) }}</span></div>
                    <div class="info-row"><span class="info-label">磁盘</span><span class="info-value">{{ formatBytes(nodeDetail.disk_free) }} / {{ formatBytes(nodeDetail.disk_total) }}</span></div>
                    <div class="info-row"><span class="info-label">运行时长</span><span class="info-value">{{ formatUptime(nodeDetail.uptime) || '-' }}</span></div>
                    <div class="info-row"><span class="info-label">启动时间</span><span class="info-value">{{ formatBoottime(nodeDetail.boottime) || '-' }}</span></div>
                  </div>
                </div>
                <div class="info-section">
                  <h3 class="section-title">节点统计</h3>
                  <div class="stats-grid">
                    <div v-for="s in nodeStatsItems" :key="s.key" class="stats-row">
                      <span class="stats-label">{{ s.label }}</span>
                      <span class="stats-value">{{ s.count }} / {{ s.max }}</span>
                    </div>
                  </div>
                </div>
              </div>
            </template>
          </template>
        </div>

        <!-- ─── Tab 3: 状态（实时当前值） ─── -->
        <div v-show="activeTab === 'status'">
          <div v-for="group in statusGroups" :key="group.key" class="metric-section">
            <h3 class="section-title">{{ group.label }}</h3>
            <div class="metric-grid">
              <div v-for="m in group.items" :key="m.key" class="metric-card">
                <div class="metric-label">{{ m.label }}</div>
                <div class="metric-value">{{ getStat(m.key + '.count') }}</div>
                <div class="metric-sub">峰值 {{ getStat(m.key + '.max') }}</div>
              </div>
            </div>
          </div>
        </div>

        <!-- ─── Tab 4: 指标（累计值） ─── -->
        <div v-show="activeTab === 'metrics'">
          <div v-for="group in metricGroups" :key="group.key" class="metric-section">
            <h3 class="section-title">{{ group.label }}</h3>
            <div class="metric-grid">
              <div v-for="m in group.items" :key="m.key" class="metric-card">
                <div class="metric-label">{{ m.label }}</div>
                <div class="metric-value">{{ getMetric(m.key) }}</div>
              </div>
            </div>
          </div>
        </div>

    `,
    setup() {
      // 标签页
      const localeState = Vue.inject('localeState');
      function $t(key, params) {
        void localeState.version;
        return window.i18n.$t(key, params);
      }

      const tabs = Vue.computed(function() {
        void localeState.version;
        return [
          { key: 'overview', label: $t('overview.title') },
          { key: 'nodes',    label: $t('overview.nodes_title') },
          { key: 'status',   label: $t('overview.status_title') },
          { key: 'metrics',  label: $t('overview.metrics_title') },
        ];
      });
      const activeTab = ref('overview');

      const stats = ref({
        connections: 0,
        sessions: 0,
        subscriptions: 0,
        topics: 0,
        sharedSubscriptions: 0,
        retained: 0,
      });
      const nodes = ref([]);
      const nodesOnline = ref(0);
      const nodesTotal = ref(0);
      const pubRate = ref('0');
      const delRate = ref('0');
      const totalConnections = ref(0);
      const metricsData = ref({});
      const statusData = ref({});

      // 时间范围
      const timeRanges = [
        { key: '15m', label: '15m' },
        { key: '30m', label: '30m' },
        { key: '1h', label: '1h' },
        { key: '6h', label: '6h' },
        { key: '12h', label: '12h' },
        { key: '1d', label: '1d' },
        { key: '3d', label: '3d' },
        { key: '7d', label: '7d' },
        { key: '15d', label: '15d' },
      ];
      const timeRange = ref('1h');

      // 指标分组：来自 /api/v1/metrics/sum 的累计值（只增不减）
      const metricGroups = Vue.computed(function() {
        void localeState.version;
        return [
          { key: 'clients', label: '客户端生命周期', items: [
            { key: 'client.authenticate', label: '认证尝试' },
            { key: 'client.connect', label: '连接成功' },
            { key: 'client.connected', label: '已连接（累计）' },
            { key: 'client.disconnected', label: '已断开（累计）' },
            { key: 'client.subscribe', label: '订阅操作' },
            { key: 'client.unsubscribe', label: '取消订阅' },
          ]},
          { key: 'sessions', label: '会话', items: [
            { key: 'session.created', label: '创建' },
            { key: 'session.resumed', label: '恢复' },
            { key: 'session.terminated', label: '终止' },
            { key: 'session.subscribed', label: '订阅变更' },
            { key: 'session.unsubscribed', label: '取消订阅' },
          ]},
          { key: 'messages', label: $t('overview.group_messages'), items: [
            { key: 'messages.publish', label: $t('overview.messages_publish') },
            { key: 'messages.delivered', label: $t('overview.messages_delivered') },
            { key: 'messages.acked', label: '确认消息数' },
            { key: 'messages.dropped', label: $t('overview.messages_discarded') },
            { key: 'messages.nonsubscribed', label: '无订阅者丢弃' },
          ]},
        ];
      });

      // 状态分组：来自 /api/v1/stats/sum 的实时当前值
      const statusGroups = Vue.computed(function() {
        void localeState.version;
        return [
          { key: 'conn_sessions', label: $t('overview.group_connections'), items: [
            { key: 'connections', label: $t('overview.connections_count') },
            { key: 'sessions', label: $t('overview.sessions_count') },
            { key: 'handshakings', label: $t('overview.handshakings_count') },
            { key: 'handshakings_active', label: $t('overview.handshakings_active_count') },
            { key: 'handshakings_rate', label: $t('overview.handshakings_rate_count') },
          ]},
          { key: 'subs_routes', label: $t('overview.group_topics_routes'), items: [
            { key: 'subscriptions', label: $t('overview.subscriptions_count') },
            { key: 'subscriptions_shared', label: $t('overview.shared_subscriptions_count') },
            { key: 'topics', label: $t('overview.topics_count') },
            { key: 'routes', label: $t('overview.routes_count') },
          ]},
          { key: 'queues', label: $t('overview.group_queues'), items: [
            { key: 'message_queues', label: $t('overview.queues_count') },
            { key: 'out_inflights', label: $t('overview.out_inflights_count') },
            { key: 'in_inflights', label: $t('overview.in_inflights_count') },
            { key: 'forwards', label: $t('overview.forwards_count') },
          ]},
          { key: 'storage', label: $t('overview.group_storage'), items: [
            { key: 'retaineds', label: $t('overview.retained_count') },
            { key: 'message_storages', label: $t('overview.message_storages_count') },
            { key: 'delayed_publishs', label: $t('overview.delayed_publishs_count') },
          ]},
        ];
      });

      // 历史数据（用于图表渲染，来自 history API + live poll）
      const chartData = ref([]);
      let isLiveMode = false;     // 历史 API 不可用时回退纯实时模式
      const CHART_POINTS = 360;   // 图表固定点数上限
      let maxChartPoints = CHART_POINTS + 20;    // 动态上限
      let notMergeNext = false;   // 切换时间范围时强制重建图表（避免合并动画产生竖线）
      let currentMergeWindow = 5; // 当前选中时间范围的 merge_window（秒）
      let currentLatestMinutes = 1; // 获取最新一个合并点时的回溯分钟数
      let liveHistoryTimer = null; // history 轮询定时器
      let lastLiveSnapshot = null; // 上次实时 metrics 快照，用于计算速率
      let liveRates = { inRate: 0, outRate: 0, inHistory: [], outHistory: [] };

      // 兼容 metrics API 返回的 dot 格式（messages.publish）和 underscore 格式（messages_publish）
      function ms(v, key) {
        if (v == null) return 0;
        var val = v[key];
        if (val != null) return +val;
        var alt = key.indexOf('.') >= 0 ? key.replace(/\./g, '_') : key.replace(/_/g, '.');
        return +(v[alt] || 0);
      }

      let timer = null;
      let chartMsgIn = null;
      let chartMsgOut = null;
      let chartMsgDropped = null;
      let chartConnections = null;
      let chartTopics = null;
      let chartSubscriptions = null;
      let gaugeConn = null;
      let msgRatePanel = null;

      function getMetric(key) {
        var v = metricsData.value[key];
        if (v != null) return v;
        // 兼容下划线/点号格式差异（后端 Metrics to_json() 将 _ 替换为 .）
        var alt = key.indexOf('.') >= 0 ? key.replace(/\./g, '_') : key.replace(/_/g, '.');
        v = metricsData.value[alt];
        return v != null ? v : '-';
      }

      function getStat(key) {
        var v = statusData.value[key];
        if (v == null) return '-';
        // 握手速率是后端已 /100 的浮点数，保留 1 位小数
        if (typeof v === 'number' && v % 1 !== 0) return v.toFixed(1);
        return v;
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
        // 解析各时间单位
        var parts = [];
        str.replace(/(\d+)\s*(days?|hours?|minutes?|seconds?)/gi, function(m, num, unit) {
          var key = unit.toLowerCase().replace(/s$/, '');
          parts.push({ key: key, num: parseInt(num, 10) });
        });
        if (parts.length === 0) return str;
        // 找到第一个非零位置
        var startIdx = parts.findIndex(function(p) { return p.num > 0; });
        if (startIdx === -1) startIdx = parts.length - 1; // 全部为0时至少显示最后一位
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

      function formatBoottime(str) {
        if (!str) return '-';
        // "2026-07-16 12:34:55.2054752 +00:00:00" → "2026-07-16 12:34:55"
        return str.replace(/^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}).*$/, '$1');
      }

      function pad(n) { return n.toString().padStart(2, '0'); }

      function setTimeRange(key) {
        timeRange.value = key;
        notMergeNext = true;
        fetchHistory();
      }

      function getTimeParams(key) {
        if (key.endsWith('m')) {
          var m = parseInt(key);
          return { minutes: m, merge_window: 5, latest_minutes: 1, limit: 2000 };
        }
        if (key.endsWith('h')) {
          var h = parseInt(key);
          var mw = h <= 1 ? 10 : h <= 6 ? 60 : 120;
          var lm = h <= 1 ? 1 : h <= 6 ? 2 : 3;
          return { hours: h, merge_window: mw, latest_minutes: lm, limit: 2000 };
        }
        if (key.endsWith('d')) {
          var d = parseInt(key);
          var mw = d <= 1 ? 240 : d <= 3 ? 720 : d <= 7 ? 1680 : 3600;
          var lm = d <= 1 ? 5 : d <= 3 ? 20 : d <= 7 ? 30 : 70;
          return { days: d, merge_window: mw, latest_minutes: lm, limit: 2000 };
        }
        return { minutes: 60, merge_window: 10, latest_minutes: 1, limit: 2000 };
      }

      async function fetchHistory() {
        var params = getTimeParams(timeRange.value);
        currentMergeWindow = params.merge_window;
        currentLatestMinutes = params.latest_minutes;
        var qs = '';
        if (params.minutes) qs += 'minutes=' + params.minutes;
        else if (params.hours) qs += 'hours=' + params.hours;
        else if (params.days) qs += 'days=' + params.days;
        qs += '&limit=' + params.limit + '&merge_window=' + params.merge_window;

        var [statsRes, metricsRes] = await Promise.all([
          http.get('/stats/history/sum?' + qs).catch(function() { return null; }),
          http.get('/metrics/history/sum?' + qs).catch(function() { return null; }),
        ]);

        if (!statsRes || !metricsRes || statsRes.error || metricsRes.error) {
          // History 未配置 — 回退纯实时模式
          isLiveMode = true;
          chartData.value = [];
          updateCharts();
          return;
        }

        isLiveMode = false;

        // 按 ts 建立 stats 查找表
        var statsMap = {};
        if (statsRes.data) {
          statsRes.data.forEach(function(d) {
            statsMap[d.ts] = {
              connections: d['connections.count'] ?? d.connections_count ?? 0,
              topics: d['topics.count'] ?? d.topics_count ?? 0,
              subscriptions: d['subscriptions.count'] ?? d.subscriptions_count ?? 0,
            };
          });
        }

        // 按 ts 合并 metrics + stats
        var merged = [];
        if (metricsRes.data) {
          metricsRes.data.forEach(function(d) {
            var s = statsMap[d.ts] || {};
            merged.push({
              time: d.ts,
              msgIn: d['messages.publish'] ?? d.messages_publish ?? 0,
              msgOut: d['messages.delivered'] ?? d.messages_delivered ?? 0,
              msgDropped: d['messages.dropped'] ?? d.messages_dropped ?? 0,
              connections: s.connections ?? 0,
              topics: s.topics ?? 0,
              subscriptions: s.subscriptions ?? 0,
            });
          });
        }

        chartData.value = merged.reverse();
        maxChartPoints = CHART_POINTS + 20;
        updateCharts();
        // 切换范围后重新启动 history 轮询
        startLiveHistoryPolling();
      }

      // 每 merge_window 秒通过 history API 获取最新一个合并点
      async function fetchLatestHistory() {
        if (isLiveMode) return;
        var qs = 'minutes=' + currentLatestMinutes + '&limit=1&merge_window=' + currentMergeWindow;
        var [statsRes, metricsRes] = await Promise.all([
          http.get('/stats/history/sum?' + qs).catch(function() { return null; }),
          http.get('/metrics/history/sum?' + qs).catch(function() { return null; }),
        ]);
        if (!metricsRes || !metricsRes.data || metricsRes.data.length === 0) return;
        var point = metricsRes.data[metricsRes.data.length - 1];
        var s = {};
        if (statsRes && statsRes.data && statsRes.data.length > 0) {
          var sp = statsRes.data[statsRes.data.length - 1];
          s.connections = sp['connections.count'] ?? sp.connections_count ?? 0;
          s.topics = sp['topics.count'] ?? sp.topics_count ?? 0;
          s.subscriptions = sp['subscriptions.count'] ?? sp.subscriptions_count ?? 0;
        }
        chartData.value.push({
          time: point.ts,
          msgIn: point['messages.publish'] ?? point.messages_publish ?? 0,
          msgOut: point['messages.delivered'] ?? point.messages_delivered ?? 0,
          msgDropped: point['messages.dropped'] ?? point.messages_dropped ?? 0,
          connections: s.connections ?? 0,
          topics: s.topics ?? 0,
          subscriptions: s.subscriptions ?? 0,
        });
        if (chartData.value.length > maxChartPoints) {
          chartData.value.splice(0, chartData.value.length - maxChartPoints);
        }
        updateCharts();
      }

      function startLiveHistoryPolling() {
        if (liveHistoryTimer) clearInterval(liveHistoryTimer);
        if (isLiveMode) return;
        liveHistoryTimer = setInterval(fetchLatestHistory, currentMergeWindow * 1000);
      }

      const selectedNodeId = ref(null);
      const nodeDetail = ref(null);
      const nodeStats = ref({});
      const nodeDetailLoading = ref(false);
      const nodeDetailError = ref(null);

      const nodeStatsItems = Vue.computed(function() {
        void localeState.version;
        var s = nodeStats.value;
        var keys = [
          { key: 'connections', label: '连接数' },
          { key: 'sessions', label: '会话' },
          { key: 'topics', label: '主题数' },
          { key: 'subscriptions', label: '订阅' },
          { key: 'retaineds', label: '保留消息' },
          { key: 'subscriptions_shared', label: '共享订阅' },
        ];
        return keys.map(function(item) {
          var count = s[item.key + '.count'];
          var max = s[item.key + '.max'];
          return {
            key: item.key,
            label: item.label,
            count: count != null ? count : '-',
            max: max != null ? max : '-',
          };
        });
      });

      async function showNodeDetail(nodeId) {
        selectedNodeId.value = nodeId;
        nodeDetailLoading.value = true;
        nodeDetailError.value = null;
        nodeDetail.value = null;
        nodeStats.value = {};

        try {
          var [nodeRes, statsRes] = await Promise.all([
            http.get('/nodes/' + nodeId).catch(function() { return null; }),
            http.get('/stats/' + nodeId).catch(function() { return null; }),
          ]);

          if (nodeRes) {
            nodeDetail.value = nodeRes;
          } else {
            nodeDetailError.value = '节点信息获取失败';
          }

          if (statsRes && statsRes.stats) {
            nodeStats.value = statsRes.stats;
          }

          nodeDetailLoading.value = false;
        } catch (e) {
          nodeDetailError.value = e.message || '未知错误';
          nodeDetailLoading.value = false;
        }
      }

      function hideNodeDetail() {
        selectedNodeId.value = null;
        nodeDetail.value = null;
        nodeStats.value = {};
        nodeDetailError.value = null;
      }

      function refreshNodeDetail() {
        if (selectedNodeId.value) {
          showNodeDetail(selectedNodeId.value);
        }
      }

      function goToNodeList() {
        hideNodeDetail();
        activeTab.value = 'nodes';
      }

      async function fetchData() {
        try {
          var [statsData, nodesData] = await Promise.all([
            http.get('/stats/sum').catch(function() { return null; }),
            http.get('/nodes').catch(function() { return null; }),
          ]);

          if (statsData) {
            var raw = statsData.stats || statsData;
            stats.value = {
              connections: raw['connections.count'] ?? 0,
              sessions: raw['sessions.count'] ?? 0,
              subscriptions: raw['subscriptions.count'] ?? 0,
              topics: raw['topics.count'] ?? 0,
              sharedSubscriptions: raw['subscriptions_shared.count'] ?? 0,
              retained: raw['retaineds.count'] ?? 0,
            };
            // 赋值给状态 Tab 数据源
            statusData.value = raw;
            // 设备连接速率使用 handshakings_rate.count，并在仪表盘上标记 handshakings_rate.max
            var hsRate = raw['handshakings_rate.count'];
            var hsRateMax = raw['handshakings_rate.max'];
            if (hsRate != null && gaugeConn) {
              gaugeConn.update(+hsRate, hsRateMax != null ? +hsRateMax : 0);
            }
          }
          if (nodesData) {
            var list = Array.isArray(nodesData) ? nodesData : [];
            nodes.value = list;
            nodesOnline.value = list.filter(function(n) { return n.running; }).length;
            nodesTotal.value = list.length;
            totalConnections.value = list.reduce(function(s, n) { return s + (n.connections || 0); }, 0);
          }

          // 获取指标（使用 metrics/sum 汇总数据）
          var metricsSum = await http.get('/metrics/sum').catch(function() { return null; });
          if (metricsSum) {
            metricsData.value = metricsSum;

            // 纯实时模式：用 metrics/sum 追加 chartData
            if (isLiveMode) {
              chartData.value.push({
                time: Date.now(),
                msgIn: ms(metricsSum, 'messages.publish'),
                msgOut: ms(metricsSum, 'messages.delivered'),
                msgDropped: ms(metricsSum, 'messages.dropped'),
                connections: stats.value.connections,
                topics: stats.value.topics,
                subscriptions: stats.value.subscriptions,
              });
              if (chartData.value.length > CHART_POINTS) chartData.value.shift();
            }

            // 用实时 metrics 数据计算每 2s 的速率（独立于 chartData）
            var pubTotal = ms(metricsSum, 'messages.publish');
            var delTotal = ms(metricsSum, 'messages.delivered');
            if (lastLiveSnapshot) {
              var dt = (Date.now() - lastLiveSnapshot.time) / 1000;
              if (dt > 0) {
                var pubDelta = Math.max(0, pubTotal - lastLiveSnapshot.publish);
                var delDelta = Math.max(0, delTotal - lastLiveSnapshot.delivered);
                var pubRate = pubDelta / dt;
                var delRate = delDelta / dt;
                liveRates.inRate = pubRate;
                liveRates.outRate = delRate;
                liveRates.inHistory.push({ t: Date.now(), v: pubRate, c: Math.round(pubDelta) });
                liveRates.outHistory.push({ t: Date.now(), v: delRate, c: Math.round(delDelta) });
                // 保留最近 60 个点用于柱状图
                if (liveRates.inHistory.length > 60) liveRates.inHistory.shift();
                if (liveRates.outHistory.length > 60) liveRates.outHistory.shift();
              }
            }
            lastLiveSnapshot = { time: Date.now(), publish: pubTotal, delivered: delTotal };

            if (msgRatePanel) {
              msgRatePanel.update({
                inRate: +liveRates.inRate.toFixed(1),
                outRate: +liveRates.outRate.toFixed(1),
                inHistory: liveRates.inHistory,
                outHistory: liveRates.outHistory,
                publish: pubTotal,
                delivered: delTotal,
              acked: ms(metricsSum, 'messages.acked'),
            });
          }
        }

        updateCharts();
        } catch (e) {
          console.error('fetch overview error:', e);
        }
      }

      function getRangeMs(key) {
        if (key.endsWith('m')) return parseInt(key) * 60000;
        if (key.endsWith('h')) return parseInt(key) * 3600000;
        if (key.endsWith('d')) return parseInt(key) * 86400000;
        return 3600000;
      }

      function updateCharts() {
        var data = chartData.value;
        if (data.length < 2) return;

        // 切换时间范围时强制重建图表，避免合并动画产生竖线
        var notMerge = notMergeNext;
        notMergeNext = false;

        // history 模式下按选中时间范围固定 X 轴窗口，live-only 模式自动缩放
        var now = Date.now();
        var rangeMs = isLiveMode ? 0 : getRangeMs(timeRange.value);
        var xMin = rangeMs > 0 ? now - rangeMs : undefined;
        var xMax = rangeMs > 0 ? now : undefined;

        function updateLineChart(chart, seriesName, values, color, areaColor, customTooltip) {
          if (!chart) return;
          var series = {
            name: seriesName, type: 'line', data: values, smooth: true,
            lineStyle: { color: color }, itemStyle: { color: color },
            symbol: 'none',
          };
          if (areaColor) series.areaStyle = { color: areaColor };
          chart.setOption({
            animation: false,
            tooltip: customTooltip || { trigger: 'axis', textStyle: { color: '#000' }, confine: true },
            grid: { left: 42, right: 30, top: 24, bottom: 20 },
            xAxis: {
              type: 'value',
              min: xMin,
              max: xMax,
              interval: rangeMs > 0 ? rangeMs / 3 : undefined,
              axisLabel: {
                align: 'center',
                formatter: function(value) {
                  var d = new Date(value);
                  return pad(d.getMonth() + 1) + '/' + pad(d.getDate()) + ' ' + pad(d.getHours()) + ':' + pad(d.getMinutes());
                },
                color: '#8899aa',
                fontSize: 11,
              },
              axisLine: { lineStyle: { color: '#2a3245' } },
              axisTick: { show: false },
              splitLine: { show: false },
            },
            yAxis: {
              type: 'value',
              splitLine: { show: false },
              axisLabel: {
                color: '#8899aa',
                formatter: function(v) {
                  if (v === 0) return '0';
                  var abs = Math.abs(v);
                  var sign = v < 0 ? '-' : '';
                  // < 10000 直接显示整数，最多 4 位 "9999"
                  if (abs < 10000) return sign + Math.round(abs).toString();
                  // < 100000 用 k，"10.0k"~"99.9k"（5 位含小数点）
                  if (abs < 100000) {
                    return sign + (Math.floor(abs / 100) / 10).toFixed(1) + 'k';
                  }
                  // >= 100000 用 M/B/T，"X.X 单位"始终 ≤ 5 位
                  var units = ['M', 'B', 'T'];
                  var div = 1000000;
                  for (var i = 0; i < units.length; i++) {
                    var val = abs / div;
                    if (val < 100) {
                      return sign + val.toFixed(1) + units[i];
                    }
                    div *= 1000;
                  }
                  return sign + (abs / div).toFixed(1) + units[units.length - 1];
                },
              },
            },
            legend: { show: false },
            series: [series],
          }, notMerge);
        }

        // 窗口内消息总数（不再是每秒速率，因为历史和实时间隔已统一为 merge_window）
        function toRate(prev, curr, field) {
          return [curr.time, Math.max(0, (curr[field] - prev[field]))];
        }
        var msgIn = data.length > 1 ? data.slice(1).map(function(d, i) {
          return toRate(data[i], d, 'msgIn');
        }) : [];
        var msgOut = data.length > 1 ? data.slice(1).map(function(d, i) {
          return toRate(data[i], d, 'msgOut');
        }) : [];
        var msgDropped = data.length > 1 ? data.slice(1).map(function(d, i) {
          return toRate(data[i], d, 'msgDropped');
        }) : [];

        // 生成消息流 tooltip：MM/DD HH:mm:ss + N秒内消息总数
        function makeMsgTooltip(field) {
          return {
            trigger: 'axis',
            confine: true,
            formatter: function(params) {
              var p = params[0];
              if (!p || p.dataIndex == null) return '';
              var idx = p.dataIndex;
              var curr = data[idx + 1];
              var prev = data[idx];
              if (!prev || !curr) return '';
              var t = new Date(curr.time);
              var ts = pad(t.getMonth() + 1) + '/' + pad(t.getDate()) + ' ' + pad(t.getHours()) + ':' + pad(t.getMinutes()) + ':' + pad(t.getSeconds());
              var dt = Math.round((curr.time - prev.time) / 1000);
              var total = Math.max(0, Math.round(curr[field] - prev[field]));
              return ts + '<br/>' + dt + '秒内消息总数：' + total;
            }
          };
        }

        updateLineChart(chartMsgIn, $t('overview.msg_in_trend'), msgIn, '#3b82f6', null, makeMsgTooltip('msgIn'));
        updateLineChart(chartMsgOut, $t('overview.msg_out_trend'), msgOut, '#22c55e', null, makeMsgTooltip('msgOut'));
        updateLineChart(chartMsgDropped, $t('overview.msg_dropped_trend'), msgDropped, '#ef4444', null, makeMsgTooltip('msgDropped'));
        updateLineChart(chartConnections, $t('overview.connections_trend'), data.map(function(d) { return [d.time, d.connections]; }), '#f59e0b', 'rgba(245,158,11,0.1)');
        updateLineChart(chartTopics, $t('overview.topics_trend'), data.map(function(d) { return [d.time, d.topics]; }), '#8b5cf6');
        updateLineChart(chartSubscriptions, $t('overview.subscriptions_trend'), data.map(function(d) { return [d.time, d.subscriptions]; }), '#06b6d4');
      }

      onMounted(function() {
        fetchHistory();
        fetchData();
        nextTick(function() {
          gaugeConn = new window.GaugeChart(document.getElementById('gaugeConnContainer'), 'gauge.connections', 'gauge.unit');
          msgRatePanel = new window.MsgRatePanel(document.getElementById('msgRateContainer'));
          chartMsgIn = echarts.init(document.getElementById('chartMsgIn'));
          chartMsgOut = echarts.init(document.getElementById('chartMsgOut'));
          chartMsgDropped = echarts.init(document.getElementById('chartMsgDropped'));
          chartConnections = echarts.init(document.getElementById('chartConnections'));
          chartTopics = echarts.init(document.getElementById('chartTopics'));
          chartSubscriptions = echarts.init(document.getElementById('chartSubscriptions'));
          fetchData();
          startLiveHistoryPolling();
        });
        timer = setInterval(fetchData, 2000);
      });

      onUnmounted(function() {
        if (timer) clearInterval(timer);
        if (liveHistoryTimer) clearInterval(liveHistoryTimer);
        if (gaugeConn) gaugeConn.dispose();
        if (msgRatePanel) msgRatePanel.dispose();
        if (chartMsgIn) chartMsgIn.dispose();
        if (chartMsgOut) chartMsgOut.dispose();
        if (chartMsgDropped) chartMsgDropped.dispose();
        if (chartConnections) chartConnections.dispose();
        if (chartTopics) chartTopics.dispose();
        if (chartSubscriptions) chartSubscriptions.dispose();
      });

      return {
        tabs, activeTab,
        stats, nodes, nodesOnline, nodesTotal, pubRate, delRate,
        totalConnections, statusData, statusGroups, getStat,
        metricsData, metricGroups, getMetric,
        timeRanges, timeRange, setTimeRange,
        selectedNodeId, nodeDetail, nodeDetailLoading, nodeDetailError,
        nodeStatsItems, showNodeDetail, hideNodeDetail, refreshNodeDetail,
        goToNodeList,
        formatCpuLoad, formatBytes, formatUptime, formatBoottime,
        $t,
      };
    },
  });
})();
