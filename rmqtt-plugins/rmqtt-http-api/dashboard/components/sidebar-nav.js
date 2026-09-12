/* ============================================================
   RMQTT Dashboard — multi-level navigation menu component
   EMQX style sidebar: groups + collapsible sub-menus + collapsed mode + hover flyout
   ============================================================ */
;(function() {
  'use strict';

  /**
   * Navigation tree data model
   * Every group:
   *   group  : i18n key of the group name
   *   icon   : HTML entity icon
   *   children: [
   * { hash: '#/xxx', label: 'nav.xxx', icon: '...' }    <- navigable
   * { disabled: true, label: 'nav.xxx', icon: '...' }   <- greyed out (not implemented yet)
   * { group: 'nav.sub_group', children: [...] }         <- sub-group
   *   ]
   */
  window.navTree = [
    {
      group: 'nav.group_monitoring',
      icon: '&#9632;',
      children: [
        { hash: '#/',         label: 'nav.overview',       icon: '&#9733;' },
        { hash: '#/clients',  label: 'nav.clients',       icon: '&#9679;' },
        { hash: '#/subscriptions', label: 'nav.subscriptions', icon: '&#9776;' },
        { hash: '#/retains', label: 'nav.retained', icon: '&#9827;' },
        { disabled: true,     label: 'nav.delayed_publish', icon: '&#9200;' },
        { disabled: true,     label: 'nav.alarms',         icon: '&#9888;' },
      ],
    },
    {
      group: 'nav.group_access_control',
      icon: '&#128274;',
      children: [
        { disabled: true, label: 'nav.auth',      icon: '&#128274;' },
        { disabled: true, label: 'nav.acl',       icon: '&#9989;' },
        { disabled: true, label: 'nav.blacklist', icon: '&#10007;' },
        { disabled: true, label: 'nav.flapping',  icon: '&#9889;' },
      ],
    },
    {
      group: 'nav.group_integration',
      icon: '&#8644;',
      children: [
        { disabled: true, label: 'nav.webhooks',   icon: '&#8594;' },
        { disabled: true, label: 'nav.connectors', icon: '&#8644;' },
      ],
    },
    {
      group: 'nav.group_management',
      icon: '&#9881;',
      children: [
        {
          group: 'nav.group_cluster_config',
          icon: '&#9881;',
          children: [
            { disabled: true, label: 'nav.mqtt_config', icon: '&#9881;' },
            { disabled: true, label: 'nav.cluster',     icon: '&#9642;' },
            { disabled: true, label: 'nav.listeners',   icon: '&#9835;' },
            { disabled: true, label: 'nav.logs',        icon: '&#9776;' },
            { disabled: true, label: 'nav.monitoring',  icon: '&#9654;' },
          ],
        },
        {
          group: 'nav.group_mqtt_advanced',
          icon: '&#9881;',
          children: [
            { disabled: true, label: 'nav.topic_rewrite',  icon: '&#9998;' },
            { disabled: true, label: 'nav.auto_subscribe', icon: '&#9830;' },
            { disabled: true, label: 'nav.delayed_publish', icon: '&#9200;' },
          ],
        },
        {
          group: 'nav.group_plugin_ext',
          icon: '&#9881;',
          children: [
            { disabled: true, label: 'nav.hooks',   icon: '&#9878;' },
            { hash: '#/plugins', label: 'nav.plugins', icon: '&#9881;' },
          ],
        },
      ],
    },
    {
      group: 'nav.group_diagnostics',
      icon: '&#128269;',
      children: [
        { disabled: true, label: 'nav.ws_client',    icon: '&#128172;' },
        { disabled: true, label: 'nav.topics',       icon: '&#9830;' },
        { disabled: true, label: 'nav.slow_sub',     icon: '&#9776;' },
        { disabled: true, label: 'nav.trace',        icon: '&#128269;' },
        { disabled: true, label: 'nav.drop_analysis', icon: '&#128200;' },
      ],
    },
    {
      group: 'nav.group_system',
      icon: '&#9878;',
      children: [
        { disabled: true, label: 'nav.users',     icon: '&#128100;' },
        { disabled: true, label: 'nav.audit_log', icon: '&#128196;' },
        { disabled: true, label: 'nav.api_keys',  icon: '&#128273;' },
      ],
    },
  ];

  /**
   * SidebarNav component
   * props: collapsed, currentHash
   * emits: navigate
   */
  window.SidebarNav = {
    name: 'SidebarNav',
    props: {
      collapsed: { type: Boolean, default: false },
      currentHash: { type: String, default: '' },
      localeVersion: { type: Number, default: 0 },
    },
    emits: ['navigate', 'toggle-collapse'],
    data: function() {
      return {
        expandedGroups: {},   // group collapse state { groupKey: true/false }
        hoveredGroup: null,   // group hovered in collapsed mode
        hoveredItem: null,    // menu item hovered in collapsed mode
      };
    },
    // delayed close timer for collapsed-mode hover (kept out of the shared state)
    _hoverTimer: null,
    computed: {
      // access navTree safely (window.* is not reachable from the Vue template)
      treeData: function() {
        return window.navTree || [];
      },
      // expand the first group by default (monitoring)
      defaultExpanded: function() {
        var result = {};
        var tree = this.treeData;
        if (tree && tree.length > 0) {
          result[tree[0].group] = true;
        }
        return result;
      },
    },
    methods: {
      $t: function(key) {
        return window.i18n ? window.i18n.$t(key) : key;
      },
      toggleGroup: function(groupKey) {
        this.expandedGroups[groupKey] = !this.expandedGroups[groupKey];
      },
      isExpanded: function(groupKey) {
        if (this.expandedGroups[groupKey] !== undefined) {
          return this.expandedGroups[groupKey];
        }
        return !!this.defaultExpanded[groupKey];
      },
      isActive: function(hash) {
        // Ignore the query part: with '#/clients/detail?clientid=x' the 'Clients' entry stays highlighted
        const cur = (this.currentHash || '').split('?')[0];
        return cur === hash;
      },
      isDisabled: function(item) {
        return item.disabled === true;
      },
      handleClick: function(item) {
        if (this.isDisabled(item)) return;
        if (item.hash) {
          this.$emit('navigate', item.hash);
        }
      },
      // collapsed-mode hover (delayed close so the flyout stays clickable)
      onGroupEnter: function(group) {
        if (!this.collapsed) return;
        this._clearHoverTimer();
        this.hoveredGroup = group.group;
      },
      onGroupLeave: function() {
        if (!this.collapsed) return;
        var self = this;
        this._clearHoverTimer();
        this._hoverTimer = setTimeout(function() {
          self.hoveredGroup = null;
          self.hoveredItem = null;
        }, 250);
      },
      // hover on the flyout itself (keeps it from disappearing)
      onPopupEnter: function() {
        this._clearHoverTimer();
      },
      onPopupLeave: function() {
        this.onGroupLeave();
      },
      _clearHoverTimer: function() {
        if (this._hoverTimer) {
          clearTimeout(this._hoverTimer);
          this._hoverTimer = null;
        }
      },
      onItemEnter: function(item) {
        if (this.collapsed) this.hoveredItem = item;
      },
      onItemLeave: function() {
        if (this.collapsed) this.hoveredItem = null;
      },
      // Check whether the entry is a sub-group
      isSubGroup: function(item) {
        return item.group && item.children;
      },
    },
    template: `
      <nav class="sidebar-nav" :class="{ collapsed: collapsed }">
        <!-- Hidden locale version trigger, so the menu reacts to a locale switch -->
        <span style="display:none">{{ localeVersion }}</span>
        <div class="sidebar-nav-header">
          <div v-if="!collapsed" class="sidebar-nav-brand">
            <h2>RMQTT</h2>
            <span class="sidebar-subtitle">Dashboard</span>
          </div>
          <div v-else class="sidebar-nav-brand-mini">
            <h2>R</h2>
          </div>
          <button class="sidebar-collapse-btn" @click="$emit('toggle-collapse')" :title="$t('nav.toggle_sidebar')">
            {{ collapsed ? '&#9654;' : '&#9664;' }}
          </button>
        </div>

        <!-- Navigation menu -->
        <div class="sidebar-nav-body">
          <div v-for="group in treeData" :key="group.group" class="nav-group"
               @mouseenter="onGroupEnter(group)" @mouseleave="onGroupLeave()">
            <!-- Group title -->
            <div class="nav-group-title" @click="toggleGroup(group.group)">
              <span class="nav-icon" v-show="collapsed" v-html="group.icon"></span>
              <span v-show="!collapsed" class="nav-label">{{ $t(group.group) }}</span>
              <span v-show="!collapsed" class="nav-arrow" :class="{ open: isExpanded(group.group) }">&#9660;</span>
            </div>

            <!-- Child menu list -->
            <div v-show="!collapsed && isExpanded(group.group)" class="nav-children">
              <template v-for="item in group.children" :key="item.label || item.group">
                <!-- Sub-group (third level menu) -->
                <div v-if="isSubGroup(item)" class="nav-subgroup">
                  <div class="nav-subgroup-title" @click="toggleGroup(item.group)">
                    <span class="nav-label-sub">{{ $t(item.group) }}</span>
                    <span class="nav-arrow" :class="{ open: isExpanded(item.group) }">&#9660;</span>
                  </div>
                  <div v-show="isExpanded(item.group)" class="nav-subchildren">
                    <div v-for="sub in item.children" :key="sub.label"
                         class="nav-item" :class="{ active: isActive(sub.hash), disabled: isDisabled(sub) }"
                         @click="handleClick(sub)">
                      <span class="nav-icon-small" v-show="collapsed" v-html="sub.icon"></span>
                      <span class="nav-label">{{ $t(sub.label) }}</span>
                      <span v-if="isDisabled(sub)" class="nav-badge-coming">soon</span>
                    </div>
                  </div>
                </div>
                <!-- Regular menu entry -->
                <div v-else class="nav-item"
                     :class="{ active: isActive(item.hash), disabled: isDisabled(item) }"
                     @click="handleClick(item)">
                  <span class="nav-icon" v-show="collapsed" v-html="item.icon"></span>
                  <span class="nav-label">{{ $t(item.label) }}</span>
                  <span v-if="isDisabled(item)" class="nav-badge-coming">soon</span>
                </div>
              </template>
            </div>
          </div>
        </div>

        <!-- Flyout for collapsed-mode hover -->
        <div v-if="collapsed && hoveredGroup" class="sidebar-hover-popup"
             @mouseenter="onPopupEnter" @mouseleave="onPopupLeave">
          <div class="popup-title">{{ $t(hoveredGroup) }}</div>
          <template v-for="group in treeData" :key="group.group">
            <template v-if="group.group === hoveredGroup">
              <div v-for="item in group.children" :key="item.label || item.group" class="popup-item-wrap">
                <div v-if="isSubGroup(item)" class="popup-subgroup">
                  <div class="popup-subgroup-title">{{ $t(item.group) }}</div>
                  <div v-for="sub in item.children" :key="sub.label"
                       class="popup-item" :class="{ active: isActive(sub.hash), disabled: isDisabled(sub) }"
                       @click="handleClick(sub)">
                    {{ $t(sub.label) }}
                  </div>
                </div>
                <div v-else class="popup-item" :class="{ active: isActive(item.hash), disabled: isDisabled(item) }"
                     @click="handleClick(item)">
                  {{ $t(item.label) }}
                </div>
              </div>
            </template>
          </template>
        </div>
      </nav>
    `,
  };
})();
