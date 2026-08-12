/* ============================================================
   RMQTT Dashboard — 保留消息页
   查询：topic_filter + 分页（offset/limit，上一页/下一页）
   展示：payload UTF-8 优先 / hex 兜底；详情弹窗
   ============================================================ */
window.RetainsPage = Vue.defineComponent({
  name: 'RetainsPage',
  template: `
    <div>
      <!-- 功能未启用提示 -->
      <div v-if="featureDisabled" class="features-alert" style="margin-bottom:16px;">
        {{ $t('retains.not_enabled') }}
      </div>

      <!-- 查询栏 -->
      <div class="search-bar">
        <div class="search-row">
          <input class="form-input" v-model="topicFilter"
                 :placeholder="$t('retains.topic_filter_placeholder')"
                 @keyup.enter="search" style="flex:1;min-width:200px;" />
          <select class="form-select" v-model="pageSize" style="width:110px;" @change="search">
            <option v-for="n in [10,50,100,500]" :key="n" :value="n">{{ $t('clients.limit') }}: {{ n }}</option>
          </select>
          <button class="btn btn-primary" @click="search">&#128269; {{ $t('retains.search') }}</button>
          <button class="btn" style="border:1px solid var(--border);background:transparent;color:var(--text-muted);"
                  @click="reset">&#8635; {{ $t('clients.reset') }}</button>
        </div>
      </div>

      <!-- 分页条 -->
      <div class="pager-bar" v-if="items.length > 0 || offset > 0">
        <button class="btn" :disabled="offset === 0" @click="prevPage">&#9664; {{ $t('retains.prev') }}</button>
        <span class="pager-info">
          {{ $t('retains.page_range', { start: offset + 1, end: offset + items.length }) }}
          <span v-if="loading">...</span>
          <span v-else>{{ hasMore ? $t('retains.page_more') : $t('retains.page_end') }}</span>
        </span>
        <button class="btn" :disabled="!hasMore" @click="nextPage">{{ $t('retains.next') }} &#9654;</button>
      </div>

      <!-- 列表 -->
      <div class="table-wrap" style="overflow-x:auto;">
        <table style="min-width:900px;">
          <thead>
            <tr>
              <th style="min-width:160px;">{{ $t('retains.topic') }}</th>
              <th style="min-width:100px;">{{ $t('retains.client_id') }}</th>
              <th style="width:50px;">QoS</th>
              <th style="width:90px;">{{ $t('retains.ttl') }}</th>
              <th style="width:140px;">{{ $t('retains.publish_time') }}</th>
              <th style="width:150px;text-align:center;">{{ $t('retains.action') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in items" :key="item.topic" class="clickable-row" @click="showDetail(item)">
              <td><code style="font-size:12px;">{{ item.topic }}</code></td>
              <td style="font-size:12px;">{{ item.client_id || '-' }}</td>
              <td style="text-align:center;">{{ item.publish?.qos ?? '-' }}</td>
              <td style="text-align:center;font-size:12px;">
                {{ item.remaining_ttl != null ? item.remaining_ttl + 's' : '-' }}
              </td>
              <td style="text-align:center;font-size:12px;">{{ formatTime(item.publish?.create_time) }}</td>
              <td>
                <div style="display:inline-flex;gap:6px;">
                  <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;color:var(--accent);"
                          @click.stop="showDetail(item)">&#128065; {{ $t('retains.view') }}</button>
                  <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;color:#e74c3c;"
                          @click.stop="removeRetain(item)">&#128465; {{ $t('retains.delete') }}</button>
                </div>
              </td>
            </tr>
            <tr v-if="!loading && items.length === 0">
              <td colspan="6" style="text-align:center;color:var(--text-muted);padding:40px;">{{ $t('retains.no_results') }}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <!-- 详情弹窗 -->
      <div v-if="detail" class="modal-overlay" @click.self="detail = null">
        <div class="modal-panel">
          <div class="modal-header">
            <h3>{{ detail.topic }}</h3>
            <button class="btn-icon modal-close" @click="detail = null">&times;</button>
          </div>
          <div class="modal-body">
            <table class="detail-table">
              <tr><td class="dt-label">QoS</td><td>{{ detail.publish?.qos ?? '-' }}</td></tr>
              <tr><td class="dt-label">{{ $t('retains.client_id') }}</td><td>{{ detail.client_id || '-' }}</td></tr>
              <tr><td class="dt-label">{{ $t('retains.ttl') }}</td>
                <td>{{ detail.remaining_ttl != null ? detail.remaining_ttl + 's' : '-' }}</td></tr>
              <tr><td class="dt-label">{{ $t('retains.create_time') }}</td>
                <td>{{ formatTime(detail.publish?.create_time) }}</td></tr>
              <tr><td class="dt-label">{{ $t('retains.payload') }}</td>
                <td><pre class="payload-full">{{ payloadFull(detail) }}</pre></td></tr>
              <tr v-if="detail.publish?.properties"><td class="dt-label">properties</td>
                <td><pre class="payload-full">{{ formatProperties(detail.publish.properties) }}</pre></td></tr>
            </table>
          </div>
          <div class="modal-footer">
            <button class="btn btn-primary" @click="detail = null">{{ $t('node_detail.close') }}</button>
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

    // base64 → 解码信息（UTF-8 优先，二进制 hex 兜底）
    function decodePayload(b64) {
      if (!b64) return { text: '', isText: true, raw: '' };
      try {
        var bin = atob(b64);
        var bytes = new Uint8Array(bin.length);
        for (var i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        try {
          var utf8 = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
          return { text: utf8, isText: true, raw: bin };
        } catch (e) {
          var hex = '';
          var show = Math.min(bytes.length, 64);
          for (var j = 0; j < show; j++) hex += bytes[j].toString(16).padStart(2, '0');
          return { text: '0x' + hex + (bytes.length > show ? '...' : ''), isText: false, raw: bin };
        }
      } catch (e) {
        return { text: b64, isText: false, raw: '' };
      }
    }

    function payloadPreview(item) {
      var d = decodePayload(item.publish && item.publish.payload);
      if (!d.text) return '-';
      var t = d.text.length > 60 ? d.text.slice(0, 60) + '...' : d.text;
      return t;
    }

    function payloadFull(item) {
      var d = decodePayload(item.publish && item.publish.payload);
      if (!d.text) return '-';
      if (d.isText) return d.text;
      // 二进制：显示完整 hex
      var bytes = [];
      for (var i = 0; i < d.raw.length; i++) bytes.push(d.raw.charCodeAt(i).toString(16).padStart(2, '0'));
      var hex = '';
      for (var j = 0; j < bytes.length; j += 32) hex += bytes.slice(j, j + 32).join('') + '\n';
      return '0x\n' + hex;
    }

    function formatTime(ms) {
      if (ms == null) return '-';
      var d = new Date(ms);
      return isNaN(d.getTime()) ? String(ms) : d.toLocaleString();
    }

    function formatFrom(from) {
      if (!from) return '-';
      try { return JSON.stringify(from); } catch (e) { return '-'; }
    }

    function formatProperties(props) {
      if (!props) return '-';
      try { return JSON.stringify(props, null, 2); } catch (e) { return String(props); }
    }

    async function load() {
      loading.value = true;
      error.value = null;
      try {
        var params = { offset: offset.value, limit: pageSize.value };
        var tf = topicFilter.value.trim();
        if (tf) params.topic_filter = tf;
        var data = await http.get('/retains', params);
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
    }

    // 删除保留消息：确认浮层 → DELETE /retains?topic=xxx → 刷新列表
    async function removeRetain(item) {
      if (!await window.$confirm($t('retains.delete_confirm', { topic: item.topic }))) return;
      try {
        await http.del('/retains?topic=' + encodeURIComponent(item.topic));
        // 当前页仅剩 1 条且非首页 → 回退一页，避免空页
        if (items.value.length === 1 && offset.value > 0) offset.value -= pageSize.value;
        load();
      } catch (e) {
        alert($t('retains.delete_fail', { msg: e.message }));
        // 404 说明消息已被删除（如其他端操作），刷新同步
        if (e.message.indexOf('404') === 0) load();
      }
    }

    // 检测 retain 功能是否启用（请求 /features）
    async function checkFeature() {
      try {
        var data = await http.get('/features');
        var nodes = (data && Array.isArray(data.nodes)) ? data.nodes : [];
        for (var i = 0; i < nodes.length; i++) {
          var n = nodes[i];
          if (n && typeof n === 'object' && n.features) {
            featureDisabled.value = !n.features.retain;
            return;
          }
        }
      } catch (e) {
        // 请求失败不提示，避免误报
      }
    }

    Vue.onMounted(function() {
      checkFeature();
      load();
    });

    return {
      topicFilter, pageSize, offset, items, hasMore, loading, error, detail, featureDisabled,
      payloadPreview, payloadFull, formatTime, formatFrom, formatProperties,
      load, search, prevPage, nextPage, reset, showDetail, removeRetain, $t,
    };
  },
});
