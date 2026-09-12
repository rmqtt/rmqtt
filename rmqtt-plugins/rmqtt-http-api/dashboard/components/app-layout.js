/* ============================================================
   RMQTT Dashboard — app-layout component
   Page layout with sidebar and top bar, supports collapsing the sidebar
   ============================================================ */
window.AppLayout = Vue.defineComponent({
  name: 'AppLayout',
  props: {
    collapsed: { type: Boolean, default: false },
  },
  template: `
    <div class="app-layout">
      <aside class="sidebar" :class="{ collapsed: collapsed }">
        <slot name="sidebar"></slot>
      </aside>
      <div class="main-area" :class="{ 'sidebar-collapsed': collapsed }">
        <header class="topbar">
          <slot name="topbar"></slot>
        </header>
        <main class="content">
          <slot></slot>
        </main>
      </div>
    </div>
  `,
});
