/* ============================================================
   RMQTT Dashboard — ECharts Gauge 仪表盘
   支持双指针：主指针 (v) + 最大值标记指针 (maxV)
   ============================================================ */
;(function() {
  'use strict';

  function niceScale(rate) {
    if (rate <= 200) return 200;
    var e = Math.floor(Math.log10(rate));
    var b = Math.pow(10, e);
    var n = rate / b;
    if (n <= 2) return 2 * b;
    if (n <= 5) return 5 * b;
    return 10 * b;
  }

  window.GaugeChart = function(dom, titleKey, unitKey) {
    this._dom = dom;
    this._tk = titleKey || '';
    this._uk = unitKey || '';
    this._val = 0;
    this._maxVal = 0;
    this._max = 200;
    this._chart = echarts.init(dom);
    this._render();
  };

  window.GaugeChart.prototype._opt = function(v, maxV) {
    var t = window.i18n ? window.i18n.$t(this._tk) : this._tk;
    var u = window.i18n ? window.i18n.$t(this._uk) : '';
    maxV = maxV || 0;
    // 根据两个值中的较大者自动缩放刻度
    var maxForScale = Math.max(v, maxV);
    if (maxForScale > this._max * 0.8) this._max = niceScale(maxForScale);
    var tc = getComputedStyle(document.documentElement).getPropertyValue('--text').trim() || '#e1e8ed';

    var data = [{ value: v, name: t }];
    if (maxV > 0) {
      data.push({
        value: maxV,
        name: '',
        title: { show: false },
        detail: { show: false },
        itemStyle: { color: '#f59e0b' },
      });
    }

    return {
      series: [{
        type: 'gauge',
        center: ['50%', '53.5%'],
        radius: '85%',
        startAngle: 225,
        endAngle: -45,
        min: 0,
        max: this._max,
        splitNumber: 4,
        axisLine: {
          lineStyle: { width: 10, color: [[0.5,'#22c55e'],[0.85,'#f59e0b'],[1,'#ef4444']] }
        },
        title: {
          offsetCenter: [0, '-120%'],
          fontSize: 13,
          color: tc,
          fontWeight: 600,
        },
        detail: {
          valueAnimation: true,
          fontSize: 22,
          fontWeight: 'bold',
          color: '#3b82f6',
          offsetCenter: [0, '100%'],
          formatter: function(r) { return r + u; },
        },
        data: data,
      }]
    };
  };

  window.GaugeChart.prototype._render = function() {
    if (this._chart) this._chart.setOption(this._opt(this._val, this._maxVal), true);
  };

  window.GaugeChart.prototype.update = function(v, maxV) {
    this._val = v;
    if (maxV != null) this._maxVal = maxV;
    this._render();
  };

  window.GaugeChart.prototype.refresh = function() {
    this._render();
  };

  window.GaugeChart.prototype.dispose = function() {
    if (this._chart) { this._chart.dispose(); this._chart = null; }
  };
})();
