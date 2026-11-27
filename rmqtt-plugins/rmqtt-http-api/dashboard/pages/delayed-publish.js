/* ============================================================
   RMQTT Dashboard — delayed publish ($delayed) page
   Query: topic_filter + pagination (offset/limit, previous/next page)
   Display: metadata only (no payload content); 5s auto polling;
   remaining time computed from expired_time (no per-second ticking)
   ============================================================ */
window.DelayedPublishPage = Vue.defineComponent({
  name: 'DelayedPublishPage',
  template: `
    <div>
      <!-- Feature not enabled notice (shown only when ALL nodes disabled) -->
      <div v-if="featureDisabled" class="features-alert" style="margin-bottom:16px;">
        {{ $t('delayed.not_enabled') }}
      </div>

      <!-- Query bar -->
      <div class="search-bar">
        <div class="search-row">
          <input class="form-input" v-model="topicFilter"
                 :placeholder="$t('delayed.topic_filter_placeholder')"
                 @keyup.enter="search" style="flex:1;min-width:200px;" />
          <select class="form-select" v-model="pageSize" style="width:110px;" @change="search">
            <option v-for="n in [10,50,100,500]" :key="n" :value="n">{{ $t('clients.limit') }}: {{ n }}</option>
          </select>
          <button class="btn btn-primary" @click="search">&#128269; {{ $t('retains.search') }}</button>
          <button class="btn" style="border:1px solid var(--border);background:transparent;color:var(--text-muted);"
                  @click="reset">&#8635; {{ $t('clients.reset') }}</button>
        </div>
      </div>

      <!-- Pagination bar -->
      <div class="pager-bar" v-if="items.length > 0 || offset > 0">
        <button class="btn" :disabled="offset === 0" @click="prevPage">&#9664; {{ $t('retains.prev') }}</button>
        <span class="pager-info">
          {{ $t('retains.page_range', { start: offset + 1, end: offset + items.length }) }}
          <span v-if="loading">...</span>
          <span v-else>{{ hasMore ? $t('retains.page_more') : $t('retains.page_end') }}</span>
        </span>
        <button class="btn" :disabled="!hasMore" @click="nextPage">{{ $t('retains.next') }} &#9654;</button>
      </div>

      <!-- List -->
      <div class="table-wrap" style="overflow-x:auto;">
        <table style="min-width:1120px;">
          <thead>
            <tr>
              <th style="min-width:160px;">{{ $t('delayed.topic') }}</th>
              <th style="width:90px;">{{ $t('delayed.delay_interval') }}</th>
              <th style="width:140px;">{{ $t('delayed.fire_time') }}</th>
              <th style="width:110px;">{{ $t('delayed.remaining') }}</th>
              <th style="min-width:110px;">{{ $t('delayed.client_id') }}</th>
              <th style="min-width:90px;">{{ $t('delayed.username') }}</th>
              <th style="width:50px;">QoS</th>
              <th style="width:100px;">{{ $t('delayed.payload_size') }}</th>
              <th style="width:60px;">{{ $t('delayed.node') }}</th>
              <th style="width:130px;text-align:center;">{{ $t('retains.action') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in items" :key="item.node_id + '/' + item.topic + '/' + item.expired_time"
                class="clickable-row" @click="showDetail(item)">
              <td><code style="font-size:12px;">{{ item.topic }}</code></td>
              <td style="text-align:center;font-size:12px;">{{ fmtDuration(item.delay_interval) }}</td>
              <td style="text-align:center;font-size:12px;">{{ formatTime(item.expired_time) }}</td>
              <td style="text-align:center;font-size:12px;color:var(--accent);">{{ formatRemaining(item) }}</td>
              <td style="font-size:12px;">{{ item.client_id || '-' }}</td>
              <td style="font-size:12px;">{{ item.username || '-' }}</td>
              <td style="text-align:center;">{{ item.qos ?? '-' }}</td>
              <td style="text-align:center;font-size:12px;">{{ fmtSize(item.payload_len) }}</td>
              <td style="text-align:center;font-size:12px;">{{ item.node_id }}</td>
              <td style="text-align:center;">
                <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;color:var(--accent);"
                        @click.stop="showDetail(item)">&#128065; {{ $t('delayed.view_payload') }}</button>
              </td>
            </tr>
            <tr v-if="!loading && items.length === 0">
              <td colspan="10" style="text-align:center;color:var(--text-muted);padding:40px;">{{ $t('delayed.no_results') }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- Detail dialog -->
      <div v-if="detail" class="modal-overlay" @click.self="detail = null">
        <div class="modal-panel" style="width:720px;max-width:92vw;">
          <div class="modal-header">
            <h3>{{ $t('retains.view') }}</h3>
            <button class="btn-icon modal-close" @click="detail = null">&times;</button>
          </div>
          <div class="modal-body">
            <table class="detail-table">
              <tr><td class="dt-label">{{ $t('delayed.topic') }}</td><td><code>{{ detail.topic }}</code></td></tr>
            </table>
            <div style="margin:12px 0 6px;font-size:13px;color:var(--text);">Payload</div>
            <div v-if="payloadState === 'ready'" class="payload-editor">
              <div class="payload-gutter"><div v-for="n in payloadLineCount" :key="n">{{ n }}</div></div>
              <pre class="payload-content">{{ payloadDisplay }}</pre>
            </div>
            <div v-else class="payload-editor" style="display:flex;align-items:center;justify-content:center;min-height:200px;">
              <span v-if="payloadState === 'loading'" style="color:var(--text-muted);">{{ $t('common.loading') }}</span>
              <span v-else-if="payloadState === 'missing'" style="color:var(--text-muted);">{{ $t('delayed.not_found') }}</span>
              <span v-else style="color:var(--red);">{{ $t('common.error') }}</span>
            </div>
          </div>
          <div class="modal-footer" style="display:flex;justify-content:space-between;align-items:center;gap:12px;">
            <select class="form-select" v-model="payloadFormat" style="width:140px;" :disabled="payloadState !== 'ready'">
              <option value="plaintext">Plaintext</option>
              <option value="base64">Base64</option>
              <option value="b64decode">Base64 Decode</option>
              <option value="json">JSON</option>
              <option value="hex">Hex</option>
            </select>
            <button class="btn btn-primary" @click="copyPayload" :disabled="payloadState !== 'ready'">
              {{ copied ? '&#10003;' : $t('common.copy') }}
            </button>
          </div>
        </div>
      </div>
    </div>
  `,
  setup() {
    function $t(key, params) { return window.i18n.$t(key, params); }

    const topicFilter = Vue.ref('');
    const pageSize = Vue.ref(50);
    const offset = Vue.ref(0);
    const items = Vue.ref([]);
    const hasMore = Vue.ref(false);
    const loading = Vue.ref(false);
    const error = Vue.ref(null);
    const detail = Vue.ref(null);
    const featureDisabled = Vue.ref(false);
    // On-demand payload fetch states: idle / loading / ready / missing / error
    const payloadState = Vue.ref('idle');
    // Display format of the payload editor: plaintext / base64 / json / hex
    const payloadFormat = Vue.ref('plaintext');
    const payloadBase64 = Vue.ref('');
    const payloadBytes = Vue.ref(null);  // Uint8Array decoded from base64
    const copied = Vue.ref(false);
    let copyTimer = null;
    let pollTimer = null;
    let payloadSeq = 0;  // guards against out-of-order responses

    function b64ToBytes(b64) {
      var bin = atob(b64);
      var bytes = new Uint8Array(bin.length);
      for (var i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      return bytes;
    }

    function utf8Decode(bytes) {
      try { return new TextDecoder('utf-8', { fatal: false }).decode(bytes); } catch (e) { return ''; }
    }

    // Hex view: continuous lowercase hex string (e.g. 65794a68...)
    function hexDump(bytes) {
      if (!bytes || bytes.length === 0) return '(empty)';
      var hex = '';
      for (var i = 0; i < bytes.length; i++) hex += bytes[i].toString(16).padStart(2, '0');
      return hex;
    }

    // Rendered content of the payload editor, per the selected display format
    const payloadDisplay = Vue.computed(function() {
      if (payloadState.value !== 'ready') return '';
      var bytes = payloadBytes.value;
      switch (payloadFormat.value) {
        case 'base64':
          return payloadBase64.value;
        case 'b64decode': {
          // Payload content itself is base64 text: decode it back to the
          // original bytes. Invalid base64 -> show the original text;
          // binary decoded content -> hex fallback.
          var text = utf8Decode(bytes);
          try {
            var decoded = b64ToBytes(text.trim());
            try { return new TextDecoder('utf-8', { fatal: true }).decode(decoded); }
            catch (e) { return hexDump(decoded); }
          } catch (e) {
            return text;
          }
        }
        case 'json': {
          var text = utf8Decode(bytes);
          try { return JSON.stringify(JSON.parse(text), null, 2); } catch (e) { return text; }
        }
        case 'hex':
          return hexDump(bytes);
        default:
          return utf8Decode(bytes);
      }
    });

    const payloadLineCount = Vue.computed(function() {
      return payloadDisplay.value.split('\n').length;
    });

    async function copyPayload() {
      try {
        await navigator.clipboard.writeText(payloadDisplay.value);
        copied.value = true;
        if (copyTimer) clearTimeout(copyTimer);
        copyTimer = setTimeout(function() { copied.value = false; }, 1500);
      } catch (e) {
        // clipboard unavailable (non-secure context), ignore
      }
    }

    // Fetch the full payload for the clicked entry from the on-demand detail API
    async function loadPayload(item) {
      var seq = ++payloadSeq;
      payloadState.value = 'loading';
      payloadBase64.value = '';
      payloadBytes.value = null;
      payloadFormat.value = 'plaintext';
      try {
        var params = { node_id: item.node_id, topic: item.topic, expired_time: item.expired_time };
        if (item.client_id) params.client_id = item.client_id;
        var data = await http.get('/delayed_publishs/detail', params);
        if (seq !== payloadSeq) return;  // another dialog opened meanwhile
        var b64 = (data && data.payload) || '';
        payloadBase64.value = b64;
        payloadBytes.value = b64 ? b64ToBytes(b64) : new Uint8Array(0);
        payloadState.value = 'ready';
      } catch (e) {
        if (seq !== payloadSeq) return;
        var msg = (e && e.message) || '';
        payloadState.value = msg.indexOf('404') >= 0 ? 'missing' : 'error';
      }
    }

    function formatTime(ms) {
      if (ms == null) return '-';
      var d = new Date(ms);
      return isNaN(d.getTime()) ? String(ms) : d.toLocaleString();
    }

    // Humanized duration: 3d 2h 5m 30s (high zero parts hidden)
    function fmtDuration(totalSec) {
      if (totalSec == null) return '-';
      var s = Math.max(0, Math.floor(totalSec));
      if (s === 0) return '0s';
      var d = Math.floor(s / 86400); s %= 86400;
      var h = Math.floor(s / 3600); s %= 3600;
      var m = Math.floor(s / 60); s %= 60;
      var parts = [];
      if (d) parts.push(d + 'd');
      if (h) parts.push(h + 'h');
      if (m) parts.push(m + 'm');
      if (s || parts.length === 0) parts.push(s + 's');
      return parts.slice(0, 3).join(' ');
    }

    // Remaining time before the message fires (computed on render, refreshed by polling)
    function formatRemaining(item) {
      if (item == null || item.expired_time == null) return '-';
      return fmtDuration((item.expired_time - Date.now()) / 1000);
    }

    function fmtSize(n) {
      if (n == null) return '-';
      if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
      if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
      return n + ' B';
    }

    async function load() {
      loading.value = true;
      error.value = null;
      try {
        var params = { offset: offset.value, limit: pageSize.value };
        var tf = topicFilter.value.trim();
        if (tf) params.topic_filter = tf;
        var data = await http.get('/delayed_publishs', params);
        items.value = (data && Array.isArray(data.items)) ? data.items : [];
        hasMore.value = !!(data && data.has_more);
      } catch (e) {
        items.value = [];
        hasMore.value = false;
        error.value = e.message || '查询失败';
      } finally {
        loading.value = false;
      }
    }

    function search() {
      offset.value = 0;
      load();
    }

    function prevPage() {
      if (offset.value > 0) {
        offset.value = Math.max(0, offset.value - pageSize.value);
        load();
      }
    }

    function nextPage() {
      if (hasMore.value) {
        offset.value += pageSize.value;
        load();
      }
    }

    function reset() {
      topicFilter.value = '';
      offset.value = 0;
      pageSize.value = 50;
      load();
    }

    function showDetail(item) {
      detail.value = item;
      loadPayload(item);
    }

    // Auto polling: 5s while the page is visible; refresh immediately when
    // the tab becomes visible again (same pattern as overview.js)
    function startPolling() {
      if (pollTimer) return;
      pollTimer = setInterval(function() {
        if (!document.hidden) load();
      }, 5000);
    }

    function stopPolling() {
      if (pollTimer) { clearInterval(pollTimer); pollTimer = null; }
    }

    function onVisibility() {
      if (!document.hidden) load();
    }

    // Show the not-enabled banner only when ALL nodes have the feature disabled
    // (delayed_publish is a listener-level switch, nodes may be mixed)
    async function checkFeature() {
      try {
        var data = await http.get('/features');
        var nodes = (data && Array.isArray(data.nodes)) ? data.nodes : [];
        if (nodes.length === 0) { featureDisabled.value = false; return; }
        var anyEnabled = false, known = false;
        for (var i = 0; i < nodes.length; i++) {
          var n = nodes[i];
          if (n && typeof n === 'object' && n.features) {
            known = true;
            if (n.features.delayed) { anyEnabled = true; break; }
          }
        }
        featureDisabled.value = known && !anyEnabled;
      } catch (e) {
        // Do not report a failed request, avoids false alarms
        featureDisabled.value = false;
      }
    }

    Vue.onMounted(function() {
      checkFeature();
      load();
      startPolling();
      document.addEventListener('visibilitychange', onVisibility);
    });
    Vue.onUnmounted(function() {
      stopPolling();
      document.removeEventListener('visibilitychange', onVisibility);
    });

    return {
      topicFilter, pageSize, offset, items, hasMore, loading, error, detail, featureDisabled,
      payloadState, payloadFormat, payloadDisplay, payloadLineCount, copied,
      formatTime, fmtDuration, formatRemaining, fmtSize,
      load, search, prevPage, nextPage, reset, showDetail, copyPayload, $t,
    };
  },
});
