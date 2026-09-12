/* ============================================================
   RMQTT Dashboard — global confirm overlay (Promise based)
   Usage:
     const ok = await window.$confirm(message, options);
     ok === true  -> the user pressed Confirm
     ok === false -> the user pressed Cancel / close button / clicked the mask
   options (all of them optional):
     title       custom title (defaults to i18n common.confirm_title)
     confirmText label of the confirm button (defaults to i18n common.confirm)
     cancelText  label of the cancel button (defaults to i18n common.cancel)
   A throwaway Vue instance mounted on body, destroyed as soon as the overlay closes;
   only one confirm overlay may exist at a time (a repeat call returns false).
   ============================================================ */
;(function() {
  'use strict';

  let instance = null; // current overlay instance, prevents stacking

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
            localeTick.value++; // force the texts to be recomputed so a locale switch is reflected
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
