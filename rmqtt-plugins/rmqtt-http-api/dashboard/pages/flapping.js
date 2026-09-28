/* ============================================================
   RMQTT Dashboard — connection flapping page (rmqtt-flapping)
   GET    /flapping/banned?dimension=&key=&offset=&limit=
   DELETE /flapping/banned?dimension=&key=   lift a ban by hand
   The ban table is node-local: the answer describes the node that
   served the request, never the cluster.
   ============================================================ */
window.FlappingPage = Vue.defineComponent({
  name: 'FlappingPage',
  template: `
    <div>
      <!-- Not screening notice: 'available' is false both when no gate is
           installed and when the gate is installed but switched off, which is
           why it is reported separately from an empty ban list -->
      <div v-if="available === false" class="features-alert" style="margin-bottom:16px;">
        {{ $t('flapping.not_enabled') }}
      </div>

      <!-- Summary -->
      <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(220px,260px));gap:12px;margin-bottom:16px;">
        <metric-card icon="&#9889;" :label="$t('flapping.banned_count')" :value="bannedCount" color="#f59e0b"></metric-card>
      </div>

      <!-- Query bar -->
      <div class="search-bar">
        <div class="search-row">
          <select class="form-select" v-model="dimension" style="width:170px;" @change="search">
            <option value="">{{ $t('flapping.dimension_all') }}</option>
            <option v-for="d in dimensions" :key="d" :value="d">{{ dimLabel(d) }}</option>
          </select>
          <input class="form-input" v-model="keyFilter"
                 :placeholder="$t('flapping.key_placeholder')"
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
        <span style="margin-left:auto;font-size:11px;color:var(--text-muted);">{{ $t('flapping.node_local') }}</span>
      </div>

      <!-- List -->
      <div class="table-wrap" style="overflow-x:auto;">
        <table style="min-width:1100px;">
          <thead>
            <tr>
              <th style="width:90px;">{{ $t('flapping.dimension') }}</th>
              <th style="min-width:140px;">{{ $t('flapping.key') }}</th>
              <th style="width:80px;text-align:center;">{{ $t('flapping.count') }}</th>
              <th style="width:150px;text-align:center;">{{ $t('flapping.banned_at') }}</th>
              <th style="width:150px;text-align:center;">{{ $t('flapping.banned_until') }}</th>
              <th style="width:90px;text-align:center;">{{ $t('flapping.remaining') }}</th>
              <th style="min-width:130px;">{{ $t('flapping.last_clientid') }}</th>
              <th style="min-width:130px;">{{ $t('flapping.last_ipaddress') }}</th>
              <th style="width:110px;text-align:center;">{{ $t('retains.action') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="item in items" :key="item.dimension + '/' + item.key">
              <td><span style="color:var(--accent);font-size:12px;">{{ item.dimension }}</span></td>
              <td><code style="font-size:12px;" :title="item.key">{{ item.key }}</code></td>
              <td style="text-align:center;">{{ item.count }}</td>
              <td style="text-align:center;font-size:12px;color:var(--text-muted);">{{ item.banned_at || '-' }}</td>
              <td style="text-align:center;font-size:12px;color:var(--text-muted);">{{ item.banned_until || '-' }}</td>
              <td style="text-align:center;font-size:12px;color:var(--accent);">{{ fmtRemaining(item) }}</td>
              <td style="font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;"
                  :title="item.last_clientid || ''">{{ item.last_clientid || '-' }}</td>
              <td style="font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;"
                  :title="item.last_ipaddress || ''">{{ item.last_ipaddress || '-' }}</td>
              <td style="text-align:center;">
                <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;color:#e74c3c;"
                        @click.stop="unban(item)">&#128275; {{ $t('flapping.unban') }}</button>
              </td>
            </tr>
            <tr v-if="!loading && items.length === 0">
              <td colspan="9" style="text-align:center;color:var(--text-muted);padding:40px;">{{ $t('flapping.no_results') }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  `,
  setup() {
    function $t(key, params) { return window.i18n.$t(key, params); }

    // The wire names of rmqtt::flapping::Dimension
    var dimensions = ['clientid', 'username', 'peerhost'];

    var dimension = Vue.ref('');
    var keyFilter = Vue.ref('');
    var pageSize = Vue.ref(50);
    var offset = Vue.ref(0);
    var items = Vue.ref([]);
    var hasMore = Vue.ref(false);
    var bannedCount = Vue.ref(null);
    // null = no answer yet, so a failed first load shows no notice either way
    var available = Vue.ref(null);
    var loading = Vue.ref(false);
    var error = Vue.ref(null);

    // The answer carries remaining_ms as a snapshot taken when it was built.
    // The countdown ticks locally against that anchor, so the number moves
    // every second while the requests stay at one per poll.
    var fetchedAtMs = Date.now();
    var nowMs = Vue.ref(Date.now());
    var pollTimer = null;
    var tickTimer = null;

    function dimLabel(dim) {
      return $t('flapping.dimension_' + dim);
    }

    function remainingMs(item) {
      var left = (item.remaining_ms || 0) - (nowMs.value - fetchedAtMs);
      return left > 0 ? left : 0;
    }

    // Humanized duration: 3d 2h 5m 30s (the zero parts on the left are hidden)
    function fmtDuration(ms) {
      if (ms == null) return '-';
      var s = Math.max(0, Math.floor(ms / 1000));
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

    function fmtRemaining(item) {
      return fmtDuration(remainingMs(item));
    }

    async function load() {
      loading.value = true;
      error.value = null;
      try {
        var params = { offset: offset.value, limit: pageSize.value };
        if (dimension.value) params.dimension = dimension.value;
        var k = keyFilter.value.trim();
        if (k) params.key = k;
        var data = await http.get('/flapping/banned', params);
        fetchedAtMs = Date.now();
        nowMs.value = fetchedAtMs;
        items.value = (data && Array.isArray(data.items)) ? data.items : [];
        hasMore.value = !!(data && data.has_more);
        bannedCount.value = (data && data.banned_count != null) ? data.banned_count : items.value.length;
        available.value = !!(data && data.available);
      } catch (e) {
        items.value = [];
        hasMore.value = false;
        // 'available' keeps its previous value: a failed request says nothing
        // about whether the gate is screening
        error.value = e.message || 'query failed';
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
      dimension.value = '';
      keyFilter.value = '';
      offset.value = 0;
      pageSize.value = 50;
      load();
    }

    // Lift one ban by hand. The key goes in the query string, not in the path:
    // a ClientId may contain '/'. 404 means the ban is already gone, which is
    // an outcome the operator wanted anyway, so it only triggers a refresh.
    async function unban(item) {
      if (!await window.$confirm($t('flapping.unban_confirm', { dimension: item.dimension, key: item.key }))) return;
      try {
        await http.del('/flapping/banned?dimension=' + encodeURIComponent(item.dimension)
          + '&key=' + encodeURIComponent(item.key));
        // Only one row left on the current page and it is not the first page ->
        // step back one page to avoid an empty page
        if (items.value.length === 1 && offset.value > 0) offset.value -= pageSize.value;
        load();
      } catch (e) {
        var msg = (e && e.message) || '';
        if (msg.indexOf('404') === 0) {
          load();
          return;
        }
        alert($t('flapping.unban_fail', { msg: msg }));
      }
    }

    // Bans lapse on their own, so the list is polled while the page is visible
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

    function startTicking() {
      if (tickTimer) return;
      tickTimer = setInterval(function() {
        if (!document.hidden) nowMs.value = Date.now();
      }, 1000);
    }

    Vue.onMounted(function() {
      load();
      startPolling();
      startTicking();
      document.addEventListener('visibilitychange', onVisibility);
    });
    Vue.onUnmounted(function() {
      stopPolling();
      if (tickTimer) { clearInterval(tickTimer); tickTimer = null; }
      document.removeEventListener('visibilitychange', onVisibility);
    });

    return {
      dimensions, dimension, keyFilter, pageSize, offset, items, hasMore,
      bannedCount, available, loading, error,
      dimLabel, fmtRemaining, load, search, prevPage, nextPage, reset, unban, $t,
    };
  },
});
