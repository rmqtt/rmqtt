/* ============================================================
   RMQTT Dashboard — 消息流入/流出速率面板（迷你条形图 + 累计统计）
   ============================================================ */
;(function() {
  'use strict';

  function fmtTime(ts) {
    if (!ts) return '';
    var d = new Date(ts);
    return ('0' + d.getHours()).slice(-2) + ':' +
           ('0' + d.getMinutes()).slice(-2) + ':' +
           ('0' + d.getSeconds()).slice(-2);
  }

  function formatSeries(values) {
    // values: [{t: timestamp, v: rate, c: count}, ...]
    var raw = (values || []).slice(-30);
    // 跳过最前面 1 根（初始差值可能包含历史累计，不可靠）
    raw = raw.slice(1);
    var arr = [];
    var padCount = 30 - raw.length;
    for (var p = 0; p < padCount; p++) arr.push({ t: 0, v: 0, c: 0 });
    for (var r = 0; r < raw.length; r++) arr.push(raw[r]);

    // 只从真实数据中取 max，排除填充零
    var vals = raw.map(function(d) { return d.v; });
    var max = vals.length > 0 ? Math.max.apply(null, vals) : 1;
    if (max <= 0) max = 1;

    return arr.map(function(d) {
      return {
        t: d.t,
        v: d.v,
        c: d.c || 0,
        h: d.v > 0 ? Math.round((d.v / max) * 100) : 0
      };
    });
  }

  function barsHtml(values, side) {
    var bars = formatSeries(values);
    var html = '';
    for (var i = 0; i < bars.length; i++) {
      html += '<div class="msg-rate-bar" data-side="' + side + '" data-idx="' + i + '" data-count="' + (bars[i].c || 0) + '" data-time="' + bars[i].t + '" style="height:' + bars[i].h + '%;"></div>';
    }
    return html;
  }

  function formatCount(n) {
    var l = window.i18n;
    var big = l ? l.$t('msg_rate.big') : '百万';
    var mid = l ? l.$t('msg_rate.mid') : '千';
    if (n >= 1000000) return (n / 1000000).toFixed(1) + big;
    if (n >= 1000) return (n / 1000).toFixed(1) + mid;
    return String(n);
  }

  /**
   * @param {HTMLElement} dom - 挂载 DOM 元素
   */
  window.MsgRatePanel = function(dom) {
    this._dom = dom;
    this._inRate = 0;
    this._outRate = 0;
    this._inHistory = [];
    this._outHistory = [];
    this._publish = 0;
    this._delivered = 0;
    this._acked = 0;

    dom.innerHTML =
      '<div class="msg-rate-tooltip" id="msgRateTt"></div>' +
      '<div class="msg-rate-top">' +
        '<div class="msg-rate-row">' +
          '<span class="msg-rate-label"></span>' +
          '<span class="msg-rate-value"></span>' +
        '</div>' +
        '<div class="msg-rate-bars in"></div>' +
      '</div>' +
      '<div class="msg-rate-bottom">' +
        '<div class="msg-rate-row">' +
          '<span class="msg-rate-label"></span>' +
          '<span class="msg-rate-value"></span>' +
        '</div>' +
        '<div class="msg-rate-bars out"></div>' +
        '<div class="msg-rate-stats">' +
          '<div class="msg-rate-stat-item"><span class="msg-rate-stat-dot dot-blue"></span><span class="stat-text"></span></div>' +
          '<div class="msg-rate-stat-item"><span class="msg-rate-stat-dot dot-indigo"></span><span class="stat-text"></span></div>' +
          '<div class="msg-rate-stat-item"><span class="msg-rate-stat-dot dot-green"></span><span class="stat-text"></span></div>' +
        '</div>' +
      '</div>';

    this._labelEls = dom.querySelectorAll('.msg-rate-label');
    this._valueEls = dom.querySelectorAll('.msg-rate-value');
    this._inBarEl = dom.querySelector('.msg-rate-top .msg-rate-bars.in');
    this._outBarEl = dom.querySelector('.msg-rate-bottom .msg-rate-bars.out');
    this._statTexts = dom.querySelectorAll('.stat-text');
    this._ttEl = dom.querySelector('#msgRateTt');

    // 委托 mouse 事件到整个面板
    var self = this;
    dom.addEventListener('mouseover', function(e) { self._onBarHover(e); });
    dom.addEventListener('mouseout', function(e) { self._onBarLeave(e); });

    this._render();
  };

  window.MsgRatePanel.prototype._onBarHover = function(e) {
    var bar = e.target;
    if (!bar || !bar.classList || !bar.classList.contains('msg-rate-bar')) {
      this._hideTt();
      return;
    }
    var count = bar.getAttribute('data-count');
    var ts = bar.getAttribute('data-time');
    if (!ts || ts === '0') { this._hideTt(); return; }

    this._ttEl.innerHTML = fmtTime(+ts) + '<br>' + count;
    this._ttEl.style.display = 'block';

    // 将 tooltip 定位到 bar 上方
    var rect = bar.getBoundingClientRect();
    var cardRect = this._dom.getBoundingClientRect();
    var top = rect.top - cardRect.top - 6;
    var left = rect.left - cardRect.left + rect.width / 2;
    this._ttEl.style.top = top + 'px';
    this._ttEl.style.left = left + 'px';
  };

  window.MsgRatePanel.prototype._onBarLeave = function(e) {
    var to = e.relatedTarget;
    if (to && to.closest && to.closest('#msgRateTt')) return;
    this._hideTt();
  };

  window.MsgRatePanel.prototype._hideTt = function() {
    if (this._ttEl) this._ttEl.style.display = 'none';
  };

  window.MsgRatePanel.prototype._render = function() {
    var inTitle = window.i18n ? window.i18n.$t('msg_rate.msg_in') : '消息流入速率';
    var outTitle = window.i18n ? window.i18n.$t('msg_rate.msg_out') : '消息流出速率';
    var unit = window.i18n ? window.i18n.$t('msg_rate.unit') : ' 条/秒';
    var l = window.i18n ? window.i18n : null;
    var labels = [
      l ? l.$t('msg_rate.publish') : '发布',
      l ? l.$t('msg_rate.delivered') : '投递',
      l ? l.$t('msg_rate.acked') : '确认',
    ];
    var values = [this._publish, this._delivered, this._acked];

    this._labelEls[0].textContent = inTitle + '：';
    this._valueEls[0].textContent = this._inRate + unit;
    this._labelEls[1].textContent = outTitle + '：';
    this._valueEls[1].textContent = this._outRate + unit;

    for (var i = 0; i < 3; i++) {
      this._statTexts[i].textContent = labels[i] + ': ' + formatCount(values[i]);
    }

    this._inBarEl.innerHTML = barsHtml(this._inHistory, 'in');
    this._outBarEl.innerHTML = barsHtml(this._outHistory, 'out');
  };

  window.MsgRatePanel.prototype.update = function(opts) {
    opts = opts || {};
    if (opts.inRate != null) this._inRate = opts.inRate;
    if (opts.outRate != null) this._outRate = opts.outRate;
    if (opts.inHistory) this._inHistory = opts.inHistory;
    if (opts.outHistory) this._outHistory = opts.outHistory;
    if (opts.publish != null) this._publish = opts.publish;
    if (opts.delivered != null) this._delivered = opts.delivered;
    if (opts.acked != null) this._acked = opts.acked;
    this._render();
  };

  window.MsgRatePanel.prototype.dispose = function() {
    if (this._dom) this._dom.innerHTML = '';
  };
})();
