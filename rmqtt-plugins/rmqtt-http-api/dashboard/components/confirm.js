/* ============================================================
   RMQTT Dashboard — 全局确认浮层（Promise 封装）
   用法：
     const ok = await window.$confirm(message, options);
     ok === true  → 用户点击「确认」
     ok === false → 用户点击「取消」/ 关闭按钮 / 点击遮罩
   选项 options（均可省略）：
     title      自定义标题（默认 i18n common.confirm_title）
     confirmText 确认按钮文案（默认 i18n common.confirm）
     cancelText  取消按钮文案（默认 i18n common.cancel）
   独立挂载到 body 的临时 Vue 实例，弹窗关闭后自动销毁；
   同一时间只允许一个确认弹窗（重复调用直接返回 false）。
   ============================================================ */
;(function() {
  'use strict';

  let instance = null; // 当前弹窗实例，防止叠加

  window.$confirm = function(message, options) {
    if (instance) return Promise.resolve(false);

    const opts = options || {};
    return new Promise(function(resolve) {
      const container = document.createElement('div');
      document.body.appendChild(container);

      const app = Vue.createApp({
        setup() {
          const localeTick = Vue.ref(0);
          const closing = Vue.ref(false);

          function onLocaleChanged() {
            localeTick.value++; // 触发文案重算，响应语言切换
          }
          Vue.onMounted(function() {
            window.addEventListener('locale-changed', onLocaleChanged);
          });
          Vue.onUnmounted(function() {
            window.removeEventListener('locale-changed', onLocaleChanged);
          });

          const title = Vue.computed(function() {
            localeTick.value;
            return opts.title || window.i18n.$t('common.confirm_title');
          });
          const confirmText = Vue.computed(function() {
            localeTick.value;
            return opts.confirmText || window.i18n.$t('common.confirm');
          });
          const cancelText = Vue.computed(function() {
            localeTick.value;
            return opts.cancelText || window.i18n.$t('common.cancel');
          });

          function close(result) {
            if (closing.value) return;
            closing.value = true;
            resolve(result);
            setTimeout(dispose, 200);
          }

          function dispose() {
            if (!instance) return;
            try { instance.app.unmount(); } catch (e) { /* noop */ }
            if (instance.container.parentNode) {
              instance.container.parentNode.removeChild(instance.container);
            }
            instance = null;
          }

          return { message, title, confirmText, cancelText, close };
        },
        template: `
          <div class="modal-overlay" @click.self="close(false)">
            <div class="modal-panel" style="width:auto;max-width:420px;">
              <div class="modal-header">
                <h3>{{ title }}</h3>
                <button class="btn-icon modal-close" @click="close(false)">&times;</button>
              </div>
              <div class="modal-body">
                <p style="margin:0;word-break:break-all;white-space:pre-wrap;line-height:1.6;">{{ message }}</p>
              </div>
              <div class="modal-footer">
                <button class="btn" @click="close(false)">{{ cancelText }}</button>
                <button class="btn btn-primary" style="margin-left:8px;" @click="close(true)">{{ confirmText }}</button>
              </div>
            </div>
          </div>
        `,
      });

      instance = { app, container };
      app.mount(container);
    });
  };
})();
