/* ============================================================
   RMQTT Dashboard — metric-card 组件
   指标卡片：支持单指标图标卡 / 双指标组合卡
   ============================================================ */
window.MetricCard = Vue.defineComponent({
  name: 'MetricCard',
  props: {
    icon: String,
    label: String,
    value: { type: [String, Number], default: '-' },
    color: { type: String, default: '#3b82f6' },
    items: { type: Array, default: null },
  },
  template: `
    <div class="metric-card" :class="{ 'multi-metric': items && items.length > 1 }">
      <template v-if="items && items.length">
        <div v-for="(item, idx) in items" :key="idx" class="metric-card-col">
          <div class="metric-card-header">
            <span class="metric-icon" :style="{ color: item.color || '#3b82f6' }">{{ item.icon || '' }}</span>
            <span class="metric-label">{{ item.label }}</span>
          </div>
          <div class="metric-value">{{ item.value != null ? item.value : '-' }}</div>
        </div>
      </template>
      <template v-else>
        <div class="metric-card-header">
          <span class="metric-icon" :style="{ color: color }">{{ icon || '' }}</span>
          <span class="metric-label">{{ label }}</span>
        </div>
        <div class="metric-value">{{ value != null ? value : '-' }}</div>
      </template>
    </div>
  `,
});
