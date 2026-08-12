/* ============================================================
   RMQTT Dashboard — HTTP API 层
   所有请求直接调用 http-api 的 /api/v1/*
   ============================================================ */
window.http = {
  async request(method, path, body, params) {
    const token = store.getToken();
    const headers = { 'Content-Type': 'application/json' };
    if (token) headers['Authorization'] = 'Bearer ' + token;

    let url = '/api/v1' + path;
    if (params) {
      const qs = Object.entries(params)
        .filter(([_, v]) => v !== undefined && v !== null && v !== '')
        .map(([k, v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v))
        .join('&');
      if (qs) url += '?' + qs;
    }

    try {
      const res = await fetch(url, {
        method,
        headers,
        body: body ? JSON.stringify(body) : undefined,
      });
      if (res.status === 401) {
        store.clearToken();
        location.hash = '#/login';
        return null;
      }
      if (!res.ok) {
        const text = await res.text();
        throw new Error(res.status + ' ' + text.slice(0, 200));
      }
      const ct = res.headers.get('content-type') || '';
      if (ct.includes('application/json')) return res.json();
      return res.text();
    } catch (e) {
      console.error('http error:', method, path, e);
      throw e;
    }
  },

  get(path, params) { return this.request('GET', path, null, params); },
  post(path, body)   { return this.request('POST', path, body); },
  put(path, body)    { return this.request('PUT', path, body); },
  del(path)          { return this.request('DELETE', path); },
};
