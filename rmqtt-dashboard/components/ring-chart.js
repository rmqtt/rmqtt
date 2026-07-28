/* ============================================================
   RMQTT Dashboard — 节点连接环形图组件 (Ring Chart)
   用 ECharts pie (环形) 展示各节点连接数分布
   ============================================================ */
;(function() {
  'use strict';

  var COLORS = ['#3b82f6', '#22c55e', '#f59e0b', '#8b5cf6', '#ef4444', '#06b6d4'];

  window.RingChart = {
    _chart: null,

    init: function(dom) {
      if (this._chart) this._chart.dispose();
      this._chart = echarts.init(dom);
      return this;
    },

    update: function(nodes) {
      if (!this._chart || !nodes || nodes.length === 0) return;

      var total = nodes.reduce(function(sum, n) { return sum + (n.connections || 0); }, 0);
      var data = nodes.map(function(n, i) {
        return {
          name: n.node_name || 'node-' + i,
          value: n.connections || 0,
        };
      });

      this._chart.setOption({
        tooltip: {
          trigger: 'item',
          formatter: '{b}: {c} ({d}%)',
          textStyle: { color: '#000' },
        },
        series: [{
          type: 'pie',
          radius: ['42%', '65%'],
          center: ['50%', '50%'],
          avoidLabelOverlap: true,
          padAngle: 1,
          itemStyle: {
            borderRadius: 4,
            borderColor: '#0f1419',
            borderWidth: 2,
          },
          label: {
            show: true,
            formatter: '{b}\n{c}',
            color: '#8899aa',
            fontSize: 11,
            lineHeight: 16,
          },
          labelLine: {
            lineStyle: { color: '#2a3245' },
          },
          emphasis: {
            label: { fontSize: 13, fontWeight: 'bold', color: '#e1e8ed' },
            itemStyle: { shadowBlur: 10, shadowOffsetX: 0, shadowColor: 'rgba(0,0,0,0.5)' },
          },
          data: data,
          color: COLORS.slice(0, data.length),
        }],
      }, true);
    },

    dispose: function() {
      if (this._chart) {
        this._chart.dispose();
        this._chart = null;
      }
    },
  };
})();
