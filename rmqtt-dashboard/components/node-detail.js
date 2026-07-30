/* ============================================================
   RMQTT Dashboard — 节点详情弹窗组件
   ============================================================ */
;(function() {
  'use strict';

  window.NodeDetail = {
    name: 'NodeDetail',
    props: {
      node: { type: Object, default: null },
      visible: { type: Boolean, default: false },
    },
    emits: ['close'],
    template: `
      <div v-if="visible && node" class="modal-overlay" @click.self="$emit('close')">
        <div class="modal-panel">
          <div class="modal-header">
            <h3>{{ node.node_name || node.name || '-' }}</h3>
            <button class="btn-icon modal-close" @click="$emit('close')">&times;</button>
          </div>
          <div class="modal-body">
            <table class="detail-table">
              <tr><td class="dt-label">{{ $t('node_detail.node_id') }}</td><td>{{ node.node_id }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.name') }}</td><td>{{ node.node_name || node.name }}</td></tr>
              <tr>
                <td class="dt-label">{{ $t('node_detail.status') }}</td>
                <td>
                  <span :class="(node.node_status === 'Running' || node.running) ? 'status-online' : 'status-offline'">
                    {{ (node.node_status === 'Running' || node.running) ? $t('node_detail.online') : $t('node_detail.offline') }}
                  </span>
                </td>
              </tr>
              <tr><td class="dt-label">{{ $t('node_detail.version') }}</td><td>{{ node.version || '-' }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.connections') }}</td><td>{{ node.connections || 0 }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.cpu') }}</td><td>{{ formatCpu(node.cpuload != null ? node.cpuload : node.load1) }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.memory') }}</td><td>{{ formatMem(node) }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.uptime') }}</td><td>{{ formatUptime(node.uptime) }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.rust_version') }}</td><td>{{ node.rustc_version || '-' }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.sys_desc') }}</td><td>{{ node.sysdescr || '-' }}</td></tr>
              <tr><td class="dt-label">{{ $t('node_detail.datetime') }}</td><td>{{ node.datetime || '-' }}</td></tr>
            </table>
          </div>
          <div class="modal-footer">
            <button class="btn btn-primary" @click="$emit('close')">{{ $t('node_detail.close') }}</button>
          </div>
        </div>
      </div>
    `,
    methods: {
      formatCpu: function(load) {
        return load != null ? load.toFixed(1) + '%' : '-';
      },
      formatUptime: function(str) {
        if (!str) return '-';
        var isZh = window.i18n && window.i18n.locale && window.i18n.locale.indexOf('zh') === 0;
        var parts = [];
        str.replace(/(\d+)\s*(days?|hours?|minutes?|seconds?)/gi, function(m, num, unit) {
          var key = unit.toLowerCase().replace(/s$/, '');
          parts.push({ key: key, num: parseInt(num, 10) });
        });
        if (parts.length === 0) return str;
        var startIdx = -1;
        for (var i = 0; i < parts.length; i++) {
          if (parts[i].num > 0) { startIdx = i; break; }
        }
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
      },
      formatMem: function(n) {
        var used = n.memory_used;
        var total = n.memory_total;
        if (used == null) return '-';
        var gb = used / (1024 * 1024 * 1024);
        var usedStr = gb >= 1 ? gb.toFixed(1) + 'G' : (used / (1024 * 1024)).toFixed(0) + 'M';
        if (total == null) return usedStr;
        var totalGb = total / (1024 * 1024 * 1024);
        var totalStr = totalGb >= 1 ? totalGb.toFixed(1) + 'G' : (total / (1024 * 1024)).toFixed(0) + 'M';
        return usedStr + ' / ' + totalStr;
      },
    },
  };
})();
