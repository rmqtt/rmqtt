/* ============================================================
   RMQTT Dashboard — 通用分页组件
   用法：app.component('pagination', window.Pagination)
   ============================================================ */
;(function() {
  'use strict';

  window.Pagination = {
    name: 'Pagination',
    props: {
      page: { type: Number, required: true },
      totalPages: { type: Number, required: true },
    },
    emits: ['page-change'],
    template: `
      <div class="pagination" v-if="totalPages > 1">
        <button class="page-btn" :disabled="page <= 1" @click="$emit('page-change', page - 1)">
          &#9664;
        </button>
        <span class="page-info">{{ page }} / {{ totalPages }}</span>
        <button class="page-btn" :disabled="page >= totalPages" @click="$emit('page-change', page + 1)">
          &#9654;
        </button>
      </div>
    `,
  };
})();
