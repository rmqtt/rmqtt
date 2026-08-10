/* ============================================================
   RMQTT Dashboard — 可国际化的日期时间选择器
   替代原生 <input type="datetime-local">（原生控件跟随浏览器语言，
   无法随页面 i18n 切换），本组件全部文案走 i18n，
   月份/星期名由 Intl 按当前语言生成。
   值格式：'YYYY-MM-DDTHH:mm'（与原生 datetime-local 一致），
   空字符串表示未选择。
   ============================================================ */
window.DatetimePicker = Vue.defineComponent({
  name: 'DatetimePicker',
  props: {
    modelValue: { type: String, default: '' },
    placeholder: { type: String, default: '' },
  },
  emits: ['update:modelValue'],
  setup(props, ctx) {
    function $t(key) { return window.i18n.$t(key); }
    const pad = n => String(n).padStart(2, '0');

    // ---- 状态 ----
    const open = Vue.ref(false);
    const viewYear = Vue.ref(0);   // 面板当前显示的年
    const viewMonth = Vue.ref(0);  // 面板当前显示的月 0-11
    const selYear = Vue.ref(0);    // 0 = 未选日期
    const selMonth = Vue.ref(0);
    const selDay = Vue.ref(0);
    const selHour = Vue.ref(0);
    const selMinute = Vue.ref(0);
    const localeVer = Vue.ref(0);  // 语言切换时重建月份/星期文案
    const root = Vue.ref(null);

    // ---- 工具 ----
    function parseModel(v) {
      if (!v) return null;
      const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(v);
      if (!m) return null;
      return { y: +m[1], mo: +m[2], d: +m[3], h: +m[4], mi: +m[5] };
    }

    // 一周起始日：优先 Intl.Locale.weekInfo（1=周一...7=周日，转成 0=周日...6=周六），缺省按语言习惯
    function getFirstDay() {
      try {
        const loc = new Intl.Locale(window.i18n.locale);
        if (loc.weekInfo && typeof loc.weekInfo.firstDay === 'number') {
          return loc.weekInfo.firstDay >= 7 ? 0 : loc.weekInfo.firstDay;
        }
      } catch (e) { /* ignore */ }
      const l = String(window.i18n.locale).toLowerCase();
      if (l.indexOf('zh') === 0 || l === 'ar' || l === 'hi' || l === 'bn') return 1;
      return 0;
    }

    // 星期表头（按当前语言 + 起始日重排）
    function buildWeekdayLabels() {
      const names = [];
      try {
        const fmt = new Intl.DateTimeFormat(window.i18n.locale, { weekday: 'short' });
        // 2026-01-04 是周日，由此生成 周日..周六 的本地名
        for (let i = 0; i < 7; i++) names.push(fmt.format(new Date(2026, 0, 4 + i)));
      } catch (e) {
        names.push('Sun','Mon','Tue','Wed','Thu','Fri','Sat');
      }
      const fd = getFirstDay();
      const out = [];
      for (let i = 0; i < 7; i++) out.push(names[(fd + i) % 7]);
      return out;
    }

    // ---- 计算属性 ----
    const weekdayLabels = Vue.computed(function() {
      void localeVer.value;
      return buildWeekdayLabels();
    });

    const monthTitle = Vue.computed(function() {
      void localeVer.value;
      const y = viewYear.value, m = viewMonth.value;
      const l = window.i18n.locale;
      if (l === 'zh-CN' || l === 'zh-TW') return y + '\u5E74' + (m + 1) + '\u6708';
      try {
        return new Intl.DateTimeFormat(l, { year: 'numeric', month: 'long' }).format(new Date(y, m, 1));
      } catch (e) {
        return y + '-' + pad(m + 1);
      }
    });

    const cells = Vue.computed(function() {
      const y = viewYear.value, m = viewMonth.value;
      const first = new Date(y, m, 1);
      const offset = (first.getDay() - getFirstDay() + 7) % 7;
      const start = new Date(y, m, 1 - offset);
      const now = new Date();
      const list = [];
      for (let i = 0; i < 42; i++) {
        const d = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
        list.push({
          y: d.getFullYear(), m: d.getMonth(), d: d.getDate(),
          inMonth: d.getMonth() === m,
          isToday: d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate(),
          isSelected: selYear.value === d.getFullYear() && selMonth.value === d.getMonth() && selDay.value === d.getDate(),
        });
      }
      return list;
    });

    const displayText = Vue.computed(function() {
      const v = parseModel(props.modelValue);
      if (!v) return '';
      return pad(v.y) + '-' + pad(v.mo) + '-' + pad(v.d) + ' ' + pad(v.h) + ':' + pad(v.mi);
    });

    // ---- 行为 ----
    function emitValue() {
      if (!selYear.value) {
        ctx.emit('update:modelValue', '');
        return;
      }
      ctx.emit('update:modelValue',
        pad(selYear.value) + '-' + pad(selMonth.value + 1) + '-' + pad(selDay.value) +
        'T' + pad(selHour.value) + ':' + pad(selMinute.value));
    }

    function toggle() {
      if (open.value) { open.value = false; return; }
      const v = parseModel(props.modelValue);
      const t = new Date();
      if (v) {
        selYear.value = v.y; selMonth.value = v.mo - 1; selDay.value = v.d;
        selHour.value = v.h; selMinute.value = v.mi;
        viewYear.value = v.y; viewMonth.value = v.mo - 1;
      } else {
        selYear.value = 0;
        selHour.value = t.getHours(); selMinute.value = t.getMinutes();
        viewYear.value = t.getFullYear(); viewMonth.value = t.getMonth();
      }
      open.value = true;
    }

    function prevMonth() {
      viewMonth.value--;
      if (viewMonth.value < 0) { viewMonth.value = 11; viewYear.value--; }
    }
    function nextMonth() {
      viewMonth.value++;
      if (viewMonth.value > 11) { viewMonth.value = 0; viewYear.value++; }
    }
    function jumpTodayView() {
      const t = new Date();
      viewYear.value = t.getFullYear();
      viewMonth.value = t.getMonth();
    }

    function pickDay(cell) {
      selYear.value = cell.y;
      selMonth.value = cell.m;
      selDay.value = cell.d;
      emitValue();
    }

    // 修改时间时若无日期，自动补今天
    function ensureDate() {
      if (selYear.value) return;
      const t = new Date();
      selYear.value = t.getFullYear();
      selMonth.value = t.getMonth();
      selDay.value = t.getDate();
      viewYear.value = t.getFullYear();
      viewMonth.value = t.getMonth();
    }
    function onHourChange(e) {
      ensureDate();
      selHour.value = +e.target.value || 0;
      emitValue();
    }
    function onMinuteChange(e) {
      ensureDate();
      selMinute.value = +e.target.value || 0;
      emitValue();
    }

    function setNow() {
      const t = new Date();
      selYear.value = t.getFullYear();
      selMonth.value = t.getMonth();
      selDay.value = t.getDate();
      selHour.value = t.getHours();
      selMinute.value = t.getMinutes();
      viewYear.value = t.getFullYear();
      viewMonth.value = t.getMonth();
      emitValue();
    }

    function clear() {
      selYear.value = 0;
      ctx.emit('update:modelValue', '');
      open.value = false;
    }

    function onDocClick(e) {
      if (open.value && root.value && !root.value.contains(e.target)) open.value = false;
    }
    function onKeydown(e) {
      if (e.key === 'Escape') open.value = false;
    }
    function onLocaleChanged() { localeVer.value++; }

    Vue.onMounted(function() {
      document.addEventListener('click', onDocClick);
      document.addEventListener('keydown', onKeydown);
      window.addEventListener('locale-changed', onLocaleChanged);
    });
    Vue.onUnmounted(function() {
      document.removeEventListener('click', onDocClick);
      document.removeEventListener('keydown', onKeydown);
      window.removeEventListener('locale-changed', onLocaleChanged);
    });

    return {
      open, viewYear, viewMonth, selYear, selMonth, selDay, selHour, selMinute,
      root, weekdayLabels, monthTitle, cells, displayText,
      toggle, prevMonth, nextMonth, jumpTodayView, pickDay,
      onHourChange, onMinuteChange, setNow, clear, pad, $t,
    };
  },
  template: `
    <div class="dtp" ref="root">
      <div class="form-input dtp-input" :class="{ active: open }" @click.stop="toggle">
        <span :class="{ 'dtp-placeholder': !displayText }">{{ displayText || placeholder }}</span>
        <span class="dtp-caret">&#9660;</span>
      </div>
      <div class="dtp-popover" v-if="open" @click.stop>
        <div class="dtp-head">
          <button class="dtp-nav" type="button" :title="$t('datetime.prev_month')" @click="prevMonth">&#8249;</button>
          <span class="dtp-title">{{ monthTitle }}</span>
          <button class="dtp-nav" type="button" :title="$t('datetime.next_month')" @click="nextMonth">&#8250;</button>
        </div>
        <div class="dtp-week">
          <span v-for="(w, i) in weekdayLabels" :key="i">{{ w }}</span>
        </div>
        <div class="dtp-grid">
          <button type="button" class="dtp-cell"
                  v-for="(c, i) in cells" :key="i"
                  :class="{ out: !c.inMonth, today: c.isToday, sel: c.isSelected }"
                  @click="pickDay(c)">{{ c.d }}</button>
        </div>
        <div class="dtp-time">
          <span class="dtp-tlbl">{{ $t('datetime.hour') }}</span>
          <select class="dtp-select" :value="pad(selHour)" @change="onHourChange">
            <option v-for="n in 24" :key="n - 1" :value="n - 1">{{ pad(n - 1) }}</option>
          </select>
          <span class="dtp-colon">:</span>
          <select class="dtp-select" :value="pad(selMinute)" @change="onMinuteChange">
            <option v-for="n in 60" :key="n - 1" :value="n - 1">{{ pad(n - 1) }}</option>
          </select>
          <button type="button" class="dtp-today" @click="jumpTodayView" :title="$t('datetime.today')">&#9679;</button>
        </div>
        <div class="dtp-foot">
          <button type="button" class="btn btn-sm" @click="clear">{{ $t('datetime.clear') }}</button>
          <button type="button" class="btn btn-sm btn-primary" @click="setNow">{{ $t('datetime.now') }}</button>
        </div>
      </div>
    </div>
  `,
});
