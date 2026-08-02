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
            <span style="flex:1;"></span>
            <select class="form-select" v-model="chartNode" style="width:160px;"
                    @change="onChartNodeChange" :title="$t('overview.node_filter')">
              <option value="">{{ $t('overview.all_nodes') }}</option>
              <option v-for="n in nodes" :key="n.node_id" :value="n.node_id">{{ n.node_name || n.node_id }}</option>
            </select>
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
              <div class="chart-card-head">
                <h3>{{ $t('overview.msg_dropped_trend') }}</h3>
                <div class="seg-tabs" role="tablist" :aria-label="$t('overview.msg_dropped_trend')">
                  <button type="button" role="tab" :aria-selected="droppedTab === 'abnormal'"
                          class="seg-btn" :class="{ active: droppedTab === 'abnormal' }"
                          @click="switchDroppedTab('abnormal')">{{ $t('overview.msg_dropped_abnormal') }}</button>
                  <button type="button" role="tab" :aria-selected="droppedTab === 'nonsub'"
                          class="seg-btn" :class="{ active: droppedTab === 'nonsub' }"
                          @click="switchDroppedTab('nonsub')">{{ $t('overview.msg_dropped_nonsub') }}</button>
                </div>
              </div>
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

        <!-- ─── Tab 5: 功能支持状态 ─── -->
        <div v-show="activeTab === 'features'">
          <div class="features-card" v-if="featuresData">
            <div class="features-card-head">
              <h3>{{ $t('overview.features_title') }}</h3>
              <span v-if="featuresConsistent !== null" class="features-consistency"
                    :class="featuresConsistent ? 'consistency-ok' : 'consistency-warn'">
                {{ featuresConsistent ? $t('overview.features_consistent') : $t('overview.features_inconsistent') }}
              </span>
            </div>
            <div v-if="featuresConsistent === false" class="features-alert">
              <div v-for="c in featuresConflicts" :key="c.feature" class="features-conflict-row">
                <span class="conflict-feature">{{ featureLabel(c.feature) }}</span>
                <span v-for="g in c.values" :key="String(g.value)" class="conflict-group"
                      :class="g.value ? 'conflict-on' : 'conflict-off'">
                  {{ g.value ? $t('common.enabled') : $t('common.disabled') }}:
                  {{ g.node_ids.join(', ') }}
                </span>
              </div>
            </div>
            <div class="features-summary">
              <div v-for="f in featureSummaryItems" :key="f.key" class="feature-item">
                <span class="feature-dot" :class="'dot-' + f.state"></span>
                <span class="feature-name">{{ f.label }}</span>
                <span class="feature-desc">{{ f.desc }}</span>
              </div>
            </div>
            <!-- 功能 × 节点 矩阵 -->
            <div class="table-wrap" style="margin-top:16px;">
              <table class="features-matrix">
                <thead>
                  <tr>
                    <th class="fm-feature-th">{{ $t('overview.features_tab_title') }}</th>
                    <th v-for="n in featureMatrixNodes" :key="n.node_id">{{ n.node_name || n.node_id }}</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="f in featureSummaryItems" :key="f.key">
                    <td class="fm-feature">{{ f.label }}</td>
                    <td v-for="n in featureMatrixNodes" :key="n.node_id" class="fm-cell">
                      <span class="feature-badge" :class="n.features[f.key] ? 'badge-on' : 'badge-off'">
                        {{ n.features[f.key] ? $t('common.enabled') : $t('common.disabled') }}
                      </span>
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
          <div v-else class="loading-text">{{ $t('common.loading') }}...</div>
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
          { key: 'features', label: $t('overview.features_tab_title') },
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
      // 功能支持状态（GET /features 返回的 FeaturesSummary 对象）
      const featuresData = ref(null);
      const FEATURES = [
        { key: 'retain',                labelKey: 'overview.features_retain' },
        { key: 'message_storage',       labelKey: 'overview.features_message_storage' },
        { key: 'session_storage',       labelKey: 'overview.features_session_storage' },
        { key: 'delayed',               labelKey: 'overview.features_delayed' },
        { key: 'shared_subscription',   labelKey: 'overview.features_shared_subscription' },
        { key: 'auto_subscription',     labelKey: 'overview.features_auto_subscription' },
      ];

      const featuresConsistent = Vue.computed(function() {
        void localeState.version;
        var d = featuresData.value;
        return d && typeof d === 'object' ? !!d.consistent : null;
      });

      const featuresConflicts = Vue.computed(function() {
        void localeState.version;
        var d = featuresData.value;
        return (d && Array.isArray(d.conflicts)) ? d.conflicts : [];
      });

      // 每个功能项的启用节点数汇总（on=全部启用 / off=全部禁用 / partial=部分启用 / unknown=无数据）
      const featureSummaryItems = Vue.computed(function() {
        void localeState.version;
        var d = featuresData.value;
        var infos = (d && Array.isArray(d.nodes))
          ? d.nodes.filter(function(n) { return n && typeof n === 'object' && n.features; })
          : [];
        var total = infos.length;
        return FEATURES.map(function(f) {
          var enabled = infos.filter(function(n) { return n.features[f.key]; }).length;
          var state = total === 0 ? 'unknown' : (enabled === total ? 'on' : (enabled === 0 ? 'off' : 'partial'));
          var desc;
          if (total === 0) {
            desc = '-';
          } else if (enabled === total) {
            desc = $t('overview.features_all_nodes');
          } else if (enabled === 0) {
            desc = $t('common.disabled');
          } else {
            desc = enabled + '/' + total + ' ' + $t('overview.features_nodes');
          }
          return { key: f.key, label: $t(f.labelKey), state: state, desc: desc };
        });
      });

      function featureLabel(key) {
        return $t('overview.features_' + key);
      }

      // 功能矩阵表的节点列（仅包含成功返回 features 的节点）
      const featureMatrixNodes = Vue.computed(function() {
        void localeState.version;
        var d = featuresData.value;
        return (d && Array.isArray(d.nodes))
          ? d.nodes.filter(function(n) { return n && typeof n === 'object' && n.features; })
          : [];
      });
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
      // 折线图节点筛选：'' = 所有节点（sum），否则具体节点（单节点 history）
      const chartNode = ref('');

      function onChartNodeChange() {
        notMergeNext = true;
        chartData.value = [];
        fetchHistory();
      }

      // 依据选中节点生成 history 接口路径与查询串
      function historyPaths(qs) {
        if (chartNode.value) {
          var id = encodeURIComponent(chartNode.value);
          return {
            stats: '/stats/history/' + id + '?' + qs,
            metrics: '/metrics/history/' + id + '?' + qs,
          };
        }
        return {
          stats: '/stats/history/sum?' + qs,
          metrics: '/metrics/history/sum?' + qs,
        };
      }

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
      // 消息丢弃面板切换：'abnormal' 异常丢弃（转发失败/过期/异常）| 'nonsub' 无订阅者丢弃
      const droppedTab = ref('abnormal');
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
          http.get(historyPaths(qs).stats).catch(function() { return null; }),
          http.get(historyPaths(qs).metrics).catch(function() { return null; }),
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
          // 丢弃最新桶：可能尚未聚合完所有节点数据，避免折线末尾出现低谷/跳变
          var histData = metricsRes.data.slice(1);
          histData.forEach(function(d) {
            var s = statsMap[d.ts] || {};
            merged.push({
              time: d.ts,
              msgIn: d['messages.publish'] ?? d.messages_publish ?? 0,
              msgOut: d['messages.delivered'] ?? d.messages_delivered ?? 0,
              msgDropped: d['messages.dropped'] ?? d.messages_dropped ?? 0,
              msgNonSub: d['messages.nonsubscribed'] ?? d.messages_nonsubscribed ?? 0,
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

      // 实时轮询每次拉取最近 N 个合并点：
      //   与已有序列重叠的 ts 以最大值为准调整该点（节点数据晚到补齐时修正），
      //   未重叠的新点按 ts 升序追加在末尾。
      const HISTORY_LATEST_POINTS = 5;

      async function fetchLatestHistory() {
        if (isLiveMode) return;
        var qs = 'minutes=' + currentLatestMinutes +
                 '&limit=' + HISTORY_LATEST_POINTS + '&merge_window=' + currentMergeWindow;
        var [statsRes, metricsRes] = await Promise.all([
          http.get(historyPaths(qs).stats).catch(function() { return null; }),
          http.get(historyPaths(qs).metrics).catch(function() { return null; }),
        ]);
        if (!metricsRes || !metricsRes.data || metricsRes.data.length === 0) return;

        // stats 按 ts 建索引，与 metrics 的合并桶对齐
        var statsByTs = {};
        if (statsRes && statsRes.data) {
          statsRes.data.forEach(function(sp) {
            statsByTs[sp.ts] = {
              connections: sp['connections.count'] ?? sp.connections_count ?? 0,
              topics: sp['topics.count'] ?? sp.topics_count ?? 0,
              subscriptions: sp['subscriptions.count'] ?? sp.subscriptions_count ?? 0,
            };
          });
        }

        // 已有序列的 ts → index 索引
        var idxByTs = {};
        for (var i = 0; i < chartData.value.length; i++) {
          idxByTs[chartData.value[i].time] = i;
        }

        // 后端返回降序（最新在前），转升序处理
        var points = metricsRes.data.slice().sort(function(a, b) { return a.ts - b.ts; });

        // 丢弃最新桶：可能尚未聚合完所有节点数据，不画半成品；下一轮它完整后再画
        points.pop();
        if (points.length === 0) return;

        var toAppend = [];

        points.forEach(function(point) {
          var s = statsByTs[point.ts] || {};
          var np = {
            time: point.ts,
            msgIn: point['messages.publish'] ?? point.messages_publish ?? 0,
            msgOut: point['messages.delivered'] ?? point.messages_delivered ?? 0,
            msgDropped: point['messages.dropped'] ?? point.messages_dropped ?? 0,
            msgNonSub: point['messages.nonsubscribed'] ?? point.messages_nonsubscribed ?? 0,
            connections: s.connections ?? 0,
            topics: s.topics ?? 0,
            subscriptions: s.subscriptions ?? 0,
          };
          var idx = idxByTs[point.ts];
          if (idx != null) {
            // 重叠：以最大值为准调整该点的位置（只大不小）
            var old = chartData.value[idx];
            chartData.value[idx] = {
              time: point.ts,
              msgIn: Math.max(old.msgIn, np.msgIn),
              msgOut: Math.max(old.msgOut, np.msgOut),
              msgDropped: Math.max(old.msgDropped, np.msgDropped),
              msgNonSub: Math.max(old.msgNonSub, np.msgNonSub),
              connections: Math.max(old.connections, np.connections),
              topics: Math.max(old.topics, np.topics),
              subscriptions: Math.max(old.subscriptions, np.subscriptions),
            };
          } else {
            toAppend.push(np);
          }
        });

        // 未重叠的新点按 ts 升序插入正确位置（保持序列单调），
        // 避免轮询返回的桶比末尾旧（如大时间范围早期点被 maxChartPoints 截断过）时乱序
        toAppend.sort(function(a, b) { return a.time - b.time; });
        toAppend.forEach(function(np) {
          var insertAt = -1;
          for (var j = 0; j < chartData.value.length; j++) {
            if (chartData.value[j].time > np.time) { insertAt = j; break; }
          }
          if (insertAt === -1) {
            chartData.value.push(np);
          } else {
            chartData.value.splice(insertAt, 0, np);
          }
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
          var [statsData, nodesData, featuresRes] = await Promise.all([
            http.get('/stats/sum').catch(function() { return null; }),
            http.get('/nodes').catch(function() { return null; }),
            http.get('/features').catch(function() { return null; }),
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
          if (featuresRes && typeof featuresRes === 'object') {
            featuresData.value = featuresRes;
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
                msgNonSub: ms(metricsSum, 'messages.nonsubscribed'),
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

      // 切换消息丢弃面板 tab：强制重建图表，避免合并动画产生竖线
      function switchDroppedTab(tab) {
        if (droppedTab.value === tab) return;
        droppedTab.value = tab;
        notMergeNext = true;
        updateCharts();
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
        // 消息丢弃面板按 tab 选择数据字段与颜色（异常丢弃 / 无订阅者丢弃）
        var droppedField = droppedTab.value === 'nonsub' ? 'msgNonSub' : 'msgDropped';
        var droppedColor = droppedTab.value === 'nonsub' ? '#f59e0b' : '#ef4444';
        var droppedName = $t(droppedTab.value === 'nonsub' ? 'overview.msg_dropped_nonsub' : 'overview.msg_dropped_abnormal');
        var msgDropped = data.length > 1 ? data.slice(1).map(function(d, i) {
          return toRate(data[i], d, droppedField);
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

        // 生成连接数/主题数/订阅数类 tooltip：MM/DD HH:mm:ss + 当前值
        function makeValueTooltip() {
          return {
            trigger: 'axis',
            confine: true,
            formatter: function(params) {
              var p = params[0];
              if (!p || p.value == null || p.value[0] == null) return '';
              var t = new Date(p.value[0]);
              var ts = pad(t.getMonth() + 1) + '/' + pad(t.getDate()) + ' ' + pad(t.getHours()) + ':' + pad(t.getMinutes()) + ':' + pad(t.getSeconds());
              var val = p.value[1];
              if (typeof val === 'number' && val % 1 !== 0) val = +val.toFixed(2);
              return ts + '<br/>' + (p.seriesName || '') + '：' + val;
            }
          };
        }

        updateLineChart(chartMsgIn, $t('overview.msg_in_trend'), msgIn, '#3b82f6', null, makeMsgTooltip('msgIn'));
        updateLineChart(chartMsgOut, $t('overview.msg_out_trend'), msgOut, '#22c55e', null, makeMsgTooltip('msgOut'));
        updateLineChart(chartMsgDropped, droppedName, msgDropped, droppedColor, null, makeMsgTooltip(droppedField));
        updateLineChart(chartConnections, $t('overview.connections_trend'), data.map(function(d) { return [d.time, d.connections]; }), '#f59e0b', 'rgba(245,158,11,0.1)', makeValueTooltip());
        updateLineChart(chartTopics, $t('overview.topics_trend'), data.map(function(d) { return [d.time, d.topics]; }), '#8b5cf6', null, makeValueTooltip());
        updateLineChart(chartSubscriptions, $t('overview.subscriptions_trend'), data.map(function(d) { return [d.time, d.subscriptions]; }), '#06b6d4', null, makeValueTooltip());
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
        droppedTab, switchDroppedTab,
        chartNode, onChartNodeChange,
        selectedNodeId, nodeDetail, nodeDetailLoading, nodeDetailError,
        nodeStatsItems, showNodeDetail, hideNodeDetail, refreshNodeDetail,
        goToNodeList,
        featuresData, featuresConsistent, featuresConflicts, featureSummaryItems, featureLabel,
        featureMatrixNodes,
        formatCpuLoad, formatBytes, formatUptime, formatBoottime,
        $t,
      };
    },
  });
})();
