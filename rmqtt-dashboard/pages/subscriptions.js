/* ============================================================
   RMQTT Dashboard — 订阅管理页
   搜索：clientid / topic / qos / share
   ============================================================ */
window.SubscriptionsPage = Vue.defineComponent({
  name: 'SubscriptionsPage',
  template: `
    <div>
      <div class="search-bar">
        <div class="search-row">
          <input class="form-input" v-model="searchClient"
                 :placeholder="$t('subscriptions.client_id')"
                 @keyup.enter="loadSubs" style="flex:1;min-width:140px;" />
          <input class="form-input" v-model="searchTopic"
                 :placeholder="$t('subscriptions.placeholder_topic_exact')"
                 @keyup.enter="loadSubs" style="flex:2;min-width:200px;" />
        </div>
        <div class="search-row" style="align-items:center;">
          <select class="form-select" v-model="searchQos" style="width:100px;">
            <option value="">{{ $t('subscriptions.qos_all') }}</option>
            <option value="0">{{ $t('subscriptions.qos_0') }}</option>
            <option value="1">{{ $t('subscriptions.qos_1') }}</option>
            <option value="2">{{ $t('subscriptions.qos_2') }}</option>
          </select>
          <input class="form-input" v-model="searchShare"
                 :placeholder="$t('subscriptions.placeholder_share')"
                 style="width:130px;" @keyup.enter="loadSubs" />
          <select class="form-select" v-model="pageSize" style="width:110px;">
            <option v-for="n in [10,50,100,500,1000]" :key="n" :value="n" :selected="n===100">
              {{ $t('clients.limit') }}: {{ n }}
            </option>
          </select>
          <button class="btn btn-primary" @click="loadSubs">&#128269; {{ $t('subscriptions.search') }}</button>
          <button class="btn" style="border:1px solid var(--border);background:transparent;color:var(--text-muted);" @click="reset">&#8635; {{ $t('clients.reset') }}</button>
        </div>
      </div>

      <div class="table-wrap" style="overflow-x:auto;">
        <table style="min-width:800px;">
          <thead>
            <tr>
              <th style="min-width:120px;">{{ $t('subscriptions.client_id') }}</th>
              <th style="min-width:130px;">{{ $t('subscriptions.topic') }}</th>
              <th style="width:40px;">QoS</th>
              <th style="min-width:80px;">{{ $t('subscriptions.share_group') }}</th>
              <th style="min-width:140px;">Address</th>
              <th style="width:60px;">{{ $t('subscriptions.node') }}</th>
              <th style="width:60px;">{{ $t('subscriptions.action') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="s in subs" :key="s.clientid + s.topic">
              <td><code style="font-size:12px;">{{ s.clientid }}</code></td>
              <td><code style="font-size:12px;">{{ s.topic }}</code></td>
              <td style="text-align:center;">{{ s.opts?.qos ?? '-' }}</td>
              <td style="font-size:12px;">{{ s.opts?.group || '' }}</td>
              <td style="font-size:12px;">{{ s.client_addr || '-' }}</td>
              <td style="font-size:12px;text-align:center;">{{ s.node_id ?? '-' }}</td>
              <td>
                <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;color:var(--red);"
                        @click="unsub(s)" :title="$t('subscriptions.unsubscribe')">
                  {{ $t('subscriptions.unsub_btn') }}
                </button>
              </td>
            </tr>
            <tr v-if="subs.length === 0">
              <td colspan="7" style="text-align:center;color:var(--text-muted);padding:40px;">{{ $t('subscriptions.no_results') }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  `,
  setup() {
    function $t(key, params) { return window.i18n.$t(key, params); }

    const searchClient = Vue.ref('');
    const searchTopic = Vue.ref('');
    const searchQos = Vue.ref('');
    const searchShare = Vue.ref('');
    const pageSize = Vue.ref(100);
    const subs = Vue.ref([]);

    async function loadSubs() {
      try {
        var params = { _limit: pageSize.value };
        if (searchClient.value.trim())     params.clientid = searchClient.value.trim();
        if (searchTopic.value.trim())      params.topic = searchTopic.value.trim();
        if (searchQos.value !== '')        params.qos = +searchQos.value;
        if (searchShare.value.trim())      params.share = searchShare.value.trim();

        const data = await http.get('/subscriptions', params);
        subs.value = Array.isArray(data) ? data : [];
      } catch (e) {
        console.error(e);
      }
    }

    function reset() {
      searchClient.value = '';
      searchTopic.value = '';
      searchQos.value = '';
      searchShare.value = '';
      pageSize.value = 100;
      loadSubs();
    }

    async function unsub(s) {
      if (!await window.$confirm($t('subscriptions.unsub_confirm', { clientId: s.clientid, topic: s.topic }))) return;
      // 先立即从本地列表移除该行，获得即时反馈（不等网络请求）
      subs.value = subs.value.filter(function(x) {
        return !(x.clientid === s.clientid && x.topic === s.topic);
      });
      try {
        await http.post('/mqtt/unsubscribe', {
          clientid: s.clientid,
          topic: s.topic,
        });
      } catch (e) {
        alert($t('subscriptions.unsubscribe_fail', { msg: e.message }));
        loadSubs(); // 失败时重载列表：若后端未删，该行会重新出现
      }
    }

    Vue.onMounted(loadSubs);

    return { searchClient, searchTopic, searchQos, searchShare,
             pageSize, subs, loadSubs, reset, unsub, $t };
  },
});
