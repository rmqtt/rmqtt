/* ============================================================
   RMQTT Dashboard — 客户端页
   搜索、列表、踢出客户端
   ============================================================ */
window.ClientsPage = Vue.defineComponent({
  name: 'ClientsPage',
  template: `
    <div>
      <div class="search-bar">
        <div class="search-row">
          <input class="form-input" v-model="clientid"
                 :placeholder="$t('clients.placeholder_cid')"
                 @keyup.enter="loadClients" style="flex:1;min-width:100px;" />
          <input class="form-input" v-model="username"
                 :placeholder="$t('clients.placeholder_user')"
                 @keyup.enter="loadClients" style="flex:1;min-width:100px;" />
          <input class="form-input" v-model="ipAddress"
                 :placeholder="$t('clients.placeholder_ip')"
                 @keyup.enter="loadClients" style="flex:1;min-width:100px;" />
        </div>
        <div class="search-row" style="align-items:center;">
          <select class="form-select" v-model="filterOnline" style="width:110px;">
            <option value="">{{ $t('clients.filter_all') }}</option>
            <option value="1">{{ $t('clients.filter_online') }}</option>
            <option value="0">{{ $t('clients.filter_offline') }}</option>
          </select>
          <select class="form-select" v-model="protoVer" style="width:120px;">
            <option value="">{{ $t('clients.filter_proto_all') }}</option>
            <option value="3">{{ $t('clients.filter_proto_3') }}</option>
            <option value="4">{{ $t('clients.filter_proto_4') }}</option>
            <option value="5">{{ $t('clients.filter_proto_5') }}</option>
          </select>
          <select class="form-select" v-model="pageSize" style="width:110px;">
            <option v-for="n in [10,50,100,500,1000]" :key="n" :value="n" :selected="n===100">
              {{ $t('clients.limit') }}: {{ n }}
            </option>
          </select>
          <button class="btn btn-primary" @click="loadClients">&#128269; {{ $t('clients.search') }}</button>
          <button class="btn" style="border:1px solid var(--border);background:transparent;color:var(--text-muted);" @click="reset">&#8635; {{ $t('clients.reset') }}</button>
        </div>

        <div class="search-advanced-toggle" @click="showAdvanced = !showAdvanced">
          <span class="search-advanced-arrow" :class="{ open: showAdvanced }">&#9654;</span>
          <span>{{ $t('clients.advanced') }}</span>
          <span v-if="advancedActiveCount" class="search-advanced-badge">{{ advancedActiveCount }}</span>
        </div>

        <div class="search-advanced-panel" v-if="showAdvanced">
          <div class="filter-row">
            <div class="filter-group">
              <div class="filter-check-input" @click="useFuzzyClientid = true">
                <input type="checkbox" id="fuzzyCid" v-model="useFuzzyClientid"
                       @click.stop
                       :aria-label="$t('clients.fuzzy_cid')" />
                <input class="form-input" v-model="fuzzyClientid"
                       :placeholder="$t('clients.fuzzy_cid')"
                       :style="!useFuzzyClientid ? { pointerEvents: 'none' } : {}"
                       :disabled="!useFuzzyClientid" @keyup.enter="loadClients" />
              </div>
            </div>
            <div class="filter-group">
              <div class="filter-check-input" @click="useFuzzyUsername = true">
                <input type="checkbox" id="fuzzyUser" v-model="useFuzzyUsername"
                       @click.stop
                       :aria-label="$t('clients.fuzzy_user')" />
                <input class="form-input" v-model="fuzzyUsername"
                       :placeholder="$t('clients.fuzzy_user')"
                       :style="!useFuzzyUsername ? { pointerEvents: 'none' } : {}"
                       :disabled="!useFuzzyUsername" @keyup.enter="loadClients" />
              </div>
            </div>
          </div>
          <div class="filter-row">
            <select class="form-select" v-model="cleanStart" style="width:150px;">
              <option value="">{{ $t('clients.clean_start') }}</option>
              <option value="true">true</option>
              <option value="false">false</option>
            </select>
            <select class="form-select" v-model="sessionPresent" style="width:170px;">
              <option value="">{{ $t('clients.session') }}</option>
              <option value="true">true</option>
              <option value="false">false</option>
            </select>
          </div>
          <div class="filter-row" style="align-items:center;">
            <span class="filter-label" style="margin-bottom:0;">{{ $t('clients.created_at') }}</span>
            <datetime-picker v-model="createdGte" style="flex:1;max-width:200px;"
                             :placeholder="$t('datetime.title')" />
            <span style="margin:0 6px;color:var(--text-muted);">~</span>
            <datetime-picker v-model="createdLte" style="flex:1;max-width:200px;"
                             :placeholder="$t('datetime.title')" />
          </div>
          <div class="filter-row" style="align-items:center;">
            <span class="filter-label" style="margin-bottom:0;">{{ $t('clients.connected_at') }}</span>
            <datetime-picker v-model="connectedGte" style="flex:1;max-width:200px;"
                             :placeholder="$t('datetime.title')" />
            <span style="margin:0 6px;color:var(--text-muted);">~</span>
            <datetime-picker v-model="connectedLte" style="flex:1;max-width:200px;"
                             :placeholder="$t('datetime.title')" />
          </div>
        </div>
      </div>

      <div class="table-wrap" style="overflow-x:auto;">
        <table style="min-width:900px;">
          <thead>
            <tr>
              <th style="min-width:120px;">{{ $t('clients.client_id') }}</th>
              <th style="min-width:80px;">{{ $t('clients.username') }}</th>
              <th style="width:60px;">{{ $t('clients.node') }}</th>
              <th style="min-width:130px;">{{ $t('clients.ip_port') }}</th>
              <th style="width:50px;">{{ $t('clients.proto_short') }}</th>
              <th style="width:40px;">{{ $t('clients.subs') }}</th>
              <th style="width:60px;">{{ $t('clients.expiry') }}</th>
              <th style="width:55px;">{{ $t('clients.ka') }}</th>
              <th style="width:70px;">{{ $t('clients.status') }}</th>
              <th style="min-width:120px;">{{ $t('clients.connected_at') }}</th>
              <th style="width:70px;">{{ $t('clients.action') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="c in clients" :key="c.clientid" style="cursor:pointer;" @click="goDetail(c.clientid)">
              <td><code style="font-size:12px;">{{ c.clientid }}</code></td>
              <td style="font-size:12px;">{{ c.username || '-' }}</td>
              <td style="font-size:12px;">{{ c.node_id ?? '-' }}</td>
              <td style="font-size:12px;">{{ c.ip_address || '-' }}<span v-if="c.port">:{{ c.port }}</span></td>
              <td style="font-size:12px;">{{ c.proto_ver ? 'MQTT ' + c.proto_ver : '-' }}</td>
              <td style="font-size:12px;text-align:center;">{{ c.subscriptions_cnt ?? '-' }}</td>
              <td style="font-size:12px;text-align:center;">{{ fmtExpiry(c.expiry_interval) }}</td>
              <td style="font-size:12px;text-align:center;">{{ c.keepalive ?? '-' }}s</td>
              <td><span :class="c.connected ? 'status-online' : 'status-offline'">
                {{ c.connected ? $t('clients.connected') : $t('clients.disconnected') }}
              </span></td>
              <td style="font-size:12px;white-space:nowrap;">{{ c.connected_at || c.created_at || '-' }}</td>
              <td>
                <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;"
                        @click.stop="kick(c.clientid)"
                        :title="$t('clients.disconnect')">
                  {{ $t('clients.disconnect') }}
                </button>
              </td>
            </tr>
            <tr v-if="clients.length === 0">
              <td colspan="11" style="text-align:center;color:var(--text-muted);padding:40px;">{{ $t('clients.no_results') }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  `,
  setup() {
    // $t in setup()
    function $t(key, params) { return window.i18n.$t(key, params); }

    // ---- 基础筛选 ----
    const clientid = Vue.ref('');
    const username = Vue.ref('');
    const ipAddress = Vue.ref('');
    const filterOnline = Vue.ref('');
    const protoVer = Vue.ref('');
    const pageSize = Vue.ref(100);

    // ---- 高级面板 ----
    const showAdvanced = Vue.ref(false);
    const useFuzzyClientid = Vue.ref(false);
    const fuzzyClientid = Vue.ref('');
    const useFuzzyUsername = Vue.ref(false);
    const fuzzyUsername = Vue.ref('');
    const cleanStart = Vue.ref('');
    const sessionPresent = Vue.ref('');
    const createdGte = Vue.ref('');
    const createdLte = Vue.ref('');
    const connectedGte = Vue.ref('');
    const connectedLte = Vue.ref('');

    const clients = Vue.ref([]);

    // 高级筛选激活的字段计数
    var advancedActiveCount = Vue.computed(function() {
      var n = 0;
      if (useFuzzyClientid.value && fuzzyClientid.value.trim()) n++;
      if (useFuzzyUsername.value && fuzzyUsername.value.trim()) n++;
      if (cleanStart.value !== '') n++;
      if (sessionPresent.value !== '') n++;
      if (createdGte.value) n++;
      if (createdLte.value) n++;
      if (connectedGte.value) n++;
      if (connectedLte.value) n++;
      return n || '';
    });

    function fmtExpiry(sec) {
      if (sec == null || sec === 0) return '\u221E';
      if (sec % 3600 === 0) return (sec / 3600) + 'h';
      if (sec % 60 === 0) return (sec / 60) + 'm';
      return sec + 's';
    }

    async function loadClients() {
      try {
        var params = { _limit: pageSize.value };

        if (useFuzzyClientid.value && fuzzyClientid.value.trim()) {
          params._like_clientid = fuzzyClientid.value.trim();
        } else if (clientid.value.trim()) {
          params.clientid = clientid.value.trim();
        }

        if (useFuzzyUsername.value && fuzzyUsername.value.trim()) {
          params._like_username = fuzzyUsername.value.trim();
        } else if (username.value.trim()) {
          params.username = username.value.trim();
        }

        if (ipAddress.value.trim())       params.ip_address = ipAddress.value.trim();
        if (filterOnline.value !== '')    params.connected = filterOnline.value === '1';
        if (protoVer.value !== '')        params.proto_ver = +protoVer.value;
        if (cleanStart.value !== '')      params.clean_start = cleanStart.value === 'true';
        if (sessionPresent.value !== '')  params.session_present = sessionPresent.value === 'true';
        if (createdGte.value)             params._gte_created_at = createdGte.value.replace('T', ' ') + ':00';
        if (createdLte.value)             params._lte_created_at = createdLte.value.replace('T', ' ') + ':00';
        if (connectedGte.value)           params._gte_connected_at = connectedGte.value.replace('T', ' ') + ':00';
        if (connectedLte.value)           params._lte_connected_at = connectedLte.value.replace('T', ' ') + ':00';

        var data = await http.get('/clients', params);
        clients.value = Array.isArray(data) ? data : [];
      } catch (e) {
        console.error(e);
      }
    }

    function reset() {
      clientid.value = '';
      username.value = '';
      ipAddress.value = '';
      filterOnline.value = '';
      protoVer.value = '';
      pageSize.value = 100;
      useFuzzyClientid.value = false;
      fuzzyClientid.value = '';
      useFuzzyUsername.value = false;
      fuzzyUsername.value = '';
      cleanStart.value = '';
      sessionPresent.value = '';
      createdGte.value = '';
      createdLte.value = '';
      connectedGte.value = '';
      connectedLte.value = '';
      loadClients();
    }

    async function kick(clientid) {
      if (!confirm($t('clients.disconnect_confirm', { clientId: clientid }))) return;
      try {
        await http.del('/clients/' + encodeURIComponent(clientid));
        loadClients();
      } catch (e) {
        alert($t('clients.disconnect_fail', { msg: e.message }));
      }
    }

    // 点击行进入客户端详情页
    function goDetail(clientid) {
      location.hash = '#/clients/detail?clientid=' + encodeURIComponent(clientid);
    }

    Vue.onMounted(function() {
      // 消费快速搜索跳转携带的 clientid（#/clients?clientid=xxx）
      var qs = location.hash.split('?')[1];
      if (qs) {
        var q = new URLSearchParams(qs).get('clientid');
        if (q) {
          clientid.value = q;
          loadClients();
          return;
        }
      }
      loadClients();
    });

    return { clientid, username, ipAddress, filterOnline, protoVer, pageSize,
             showAdvanced, useFuzzyClientid, fuzzyClientid,
             useFuzzyUsername, fuzzyUsername, cleanStart, sessionPresent,
             createdGte, createdLte, connectedGte, connectedLte, clients, advancedActiveCount,
             loadClients, reset, kick, goDetail, fmtExpiry, $t };
  },
});
