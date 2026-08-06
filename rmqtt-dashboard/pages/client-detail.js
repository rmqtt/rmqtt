/* ============================================================
   RMQTT Dashboard — 客户端详情页
   布局：连接信息 | 会话信息（左右两栏），下方当前订阅列表
   数据：GET /clients/{clientid}（跨节点）+ GET /subscriptions?clientid=
   ============================================================ */
window.ClientDetailPage = Vue.defineComponent({
  name: 'ClientDetailPage',
  template: `
    <div>
      <div class="detail-toolbar">
        <button class="btn btn-sm" style="border:1px solid var(--border);background:transparent;color:var(--text-muted);"
                @click="back">&#8592; {{ $t('clients.back') }}</button>
        <span class="detail-title"><code style="font-size:14px;">{{ clientid }}</code></span>
        <span v-if="info" class="detail-status" :class="info.connected ? 'status-online' : 'status-offline'">
          {{ info.connected ? $t('clients.connected') : $t('clients.disconnected') }}
        </span>
        <span style="flex:1;"></span>
        <button v-if="info" class="btn btn-sm btn-primary" style="background:var(--red);"
                @click="kick" :title="$t('clients.disconnect')">{{ $t('clients.disconnect') }}</button>
        <button class="btn btn-sm" style="border:1px solid var(--border);background:transparent;color:var(--text-muted);"
                @click="refresh">&#x27F3; {{ $t('common.refresh') }}</button>
      </div>

      <div v-if="loading" class="loading-text">{{ $t('common.loading') }}</div>

      <div v-else-if="notFound" class="detail-empty">
        <div style="font-size:28px;margin-bottom:12px;">&#128269;</div>
        <div>{{ $t('clients.not_found', { clientId: clientid }) }}</div>
      </div>

      <template v-else-if="info">
        <div class="detail-grid">
          <div class="info-card">
            <h3>{{ $t('clients.connection_info') }}</h3>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.status') }}</span>
              <span class="info-value plain" :class="info.connected ? 'status-online' : 'status-offline'">
                {{ info.connected ? $t('clients.connected') : $t('clients.disconnected') }}
              </span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.node') }}</span>
              <span class="info-value">{{ info.node_id ?? '-' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.client_id') }}</span>
              <span class="info-value">{{ info.clientid }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.username') }}</span>
              <span class="info-value">{{ info.username || '-' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.superuser') }}</span>
              <span class="info-value plain">{{ info.superuser ? 'true' : 'false' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.proto_short') }}</span>
              <span class="info-value plain">{{ info.proto_ver ? 'MQTT ' + info.proto_ver : '-' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.address') }}</span>
              <span class="info-value">{{ info.ip_address || '-' }}<template v-if="info.port">:{{ info.port }}</template></span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.connected_at') }}</span>
              <span class="info-value">{{ info.connected_at || '-' }}</span>
            </div>
            <div class="info-row" v-if="info.disconnected_at">
              <span class="info-label">{{ $t('clients.disconnected_at') }}</span>
              <span class="info-value">{{ info.disconnected_at }}</span>
            </div>
            <div class="info-row" v-if="info.disconnected_reason">
              <span class="info-label">{{ $t('clients.disconnect_reason') }}</span>
              <span class="info-value">{{ info.disconnected_reason }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.keepalive') }}</span>
              <span class="info-value plain">{{ info.keepalive ? info.keepalive + 's' : '-' }}</span>
            </div>
            <div class="info-row" v-if="hasWill">
              <span class="info-label">{{ $t('clients.last_will') }}</span>
              <span class="info-value">{{ fmtWill(info.last_will) }}</span>
            </div>
          </div>

          <div class="info-card">
            <h3>{{ $t('clients.session_info') }}</h3>
            <div class="info-row">
              <span class="info-label">Clean Start</span>
              <span class="info-value plain">{{ info.clean_start ? 'true' : 'false' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Session Present</span>
              <span class="info-value plain">{{ info.session_present ? 'true' : 'false' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.session_expiry') }}</span>
              <span class="info-value plain">{{ fmtExpiry(info.expiry_interval) }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.session_created') }}</span>
              <span class="info-value">{{ info.created_at || '-' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">{{ $t('clients.subs_count') }}</span>
              <span class="info-value plain">{{ info.subscriptions_cnt ?? 0 }} / {{ info.max_subscriptions ?? '-' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Inflight</span>
              <span class="info-value plain">{{ info.inflight ?? 0 }} / {{ info.max_inflight ?? '-' }}</span>
            </div>
            <div class="info-row">
              <span class="info-label">Message Queue</span>
              <span class="info-value plain">{{ info.mqueue_len ?? 0 }} / {{ info.max_mqueue ?? '-' }}</span>
            </div>
          </div>
        </div>

        <div class="table-wrap" style="overflow-x:auto;">
          <div class="detail-subs-header">
            <span>{{ $t('clients.current_subs') }} <b>({{ subs.length }})</b></span>
          </div>
          <table style="min-width:640px;">
            <thead>
              <tr>
                <th style="min-width:200px;">{{ $t('subscriptions.topic') }}</th>
                <th style="width:60px;">{{ $t('subscriptions.qos') }}</th>
                <th style="min-width:100px;">{{ $t('subscriptions.share_group') }}</th>
                <th style="width:80px;">{{ $t('subscriptions.node') }}</th>
                <th style="width:80px;">{{ $t('subscriptions.action') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="s in subs" :key="s.clientid + '|' + s.topic">
                <td><code style="font-size:12px;">{{ s.topic }}</code></td>
                <td style="text-align:center;">{{ s.opts?.qos ?? '-' }}</td>
                <td style="font-size:12px;">{{ s.opts?.group || '-' }}</td>
                <td style="font-size:12px;text-align:center;">{{ s.node_id ?? '-' }}</td>
                <td>
                  <button class="btn-icon" style="width:auto;padding:3px 10px;font-size:11px;color:var(--red);"
                          @click="unsub(s)" :title="$t('subscriptions.unsubscribe')">
                    {{ $t('subscriptions.unsub_btn') }}
                  </button>
                </td>
              </tr>
              <tr v-if="subs.length === 0">
                <td colspan="5" style="text-align:center;color:var(--text-muted);padding:32px;">{{ $t('subscriptions.no_results') }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>
    </div>
  `,
  setup() {
    function $t(key, params) { return window.i18n.$t(key, params); }

    // 从 hash 解析 clientid：'#/clients/detail?clientid=xxx'
    const qs = (location.hash.split('?')[1] || '');
    const clientid = new URLSearchParams(qs).get('clientid') || '';

    const info = Vue.ref(null);
    const subs = Vue.ref([]);
    const loading = Vue.ref(true);
    const notFound = Vue.ref(false);

    function fmtExpiry(sec) {
      if (sec == null || sec === 0) return '\u221E';
      if (sec % 3600 === 0) return (sec / 3600) + 'h';
      if (sec % 60 === 0) return (sec / 60) + 'm';
      return sec + 's';
    }

    // 遗嘱消息：{ topic, message(base64), qos, retain }
    const hasWill = Vue.computed(function() {
      return !!(info.value && info.value.last_will && info.value.last_will.topic);
    });

    function fmtWill(will) {
      if (!will) return '-';
      var s = will.topic;
      if (will.qos != null) s += ' [QoS ' + will.qos + ']';
      if (will.retain) s += ' [retain]';
      if (will.message) {
        var payload = '';
        try {
          var bin = atob(will.message);
          payload = new TextDecoder().decode(Uint8Array.from(bin, function(c) { return c.charCodeAt(0); }));
        } catch (e) {
          payload = will.message;
        }
        payload = payload.replace(/\s+/g, ' ').trim();
        if (payload.length > 40) payload = payload.slice(0, 40) + '...';
        if (payload) s += ' \u00B7 ' + payload;
      }
      return s;
    }

    async function load() {
      if (!clientid) { notFound.value = true; loading.value = false; return; }
      loading.value = true;
      notFound.value = false;
      try {
        var [detail, subList] = await Promise.all([
          http.get('/clients/' + encodeURIComponent(clientid)).catch(function(e) { return null; }),
          http.get('/subscriptions', { clientid: clientid, _limit: 1000 }).catch(function() { return []; }),
        ]);
        if (detail) {
          info.value = detail;
        } else {
          info.value = null;
          notFound.value = true;
        }
        subs.value = Array.isArray(subList) ? subList : [];
      } catch (e) {
        console.error(e);
      } finally {
        loading.value = false;
      }
    }

    // 刷新不显示全屏 loading
    function refresh() {
      Promise.all([
        http.get('/clients/' + encodeURIComponent(clientid)).catch(function() { return null; }),
        http.get('/subscriptions', { clientid: clientid, _limit: 1000 }).catch(function() { return []; }),
      ]).then(function(res) {
        var detail = res[0], subList = res[1];
        if (detail) {
          info.value = detail;
          notFound.value = false;
        } else {
          info.value = null;
          notFound.value = true;
        }
        subs.value = Array.isArray(subList) ? subList : [];
      }).catch(function(e) { console.error(e); });
    }

    async function kick() {
      if (!await window.$confirm($t('clients.disconnect_confirm', { clientId: clientid }))) return;
      try {
        await http.del('/clients/' + encodeURIComponent(clientid));
        refresh();
      } catch (e) {
        alert($t('clients.disconnect_fail', { msg: e.message }));
      }
    }

    async function unsub(s) {
      if (!await window.$confirm($t('subscriptions.unsub_confirm', { clientId: s.clientid, topic: s.topic }))) return;
      try {
        await http.post('/mqtt/unsubscribe', { clientid: s.clientid, topic: s.topic });
        refresh();
      } catch (e) {
        alert($t('subscriptions.unsubscribe_fail', { msg: e.message }));
      }
    }

    function back() {
      location.hash = '#/clients';
    }

    Vue.onMounted(load);

    return { clientid, info, subs, loading, notFound, hasWill,
             fmtExpiry, fmtWill, load, refresh, kick, unsub, back, $t };
  },
});
