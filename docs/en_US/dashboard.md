English | [简体中文](../zh_CN/dashboard.md)

# Dashboard (Web Management UI)

RMQTT ships with a built-in web management UI (`rmqtt-dashboard/`) for viewing cluster status, metrics, clients, retained messages, and more. It is a **build-free** vanilla SPA (`index.html` + vanilla JS + Vue3 / ECharts via CDN), served by the `rmqtt-http-api` plugin.

## Access

### 1. Embedded mode (default)

The Dashboard assets are embedded into the binary at compile time via `rust-embed`, so no external files are needed:

```
http://<host>:6060/dashboard/
```

Frontend changes require recompiling (`cargo build`) to take effect.

### 2. External directory mode (hot reload)

Set `dashboard_static_dir` in `rmqtt-http-api.toml` to point at the Dashboard source directory:

```toml
dashboard_static_dir = "/path/to/rmqtt-dashboard"
```

- The plugin serves the SPA from both `/` and `/dashboard/`
- **Relative paths are resolved against the process cwd** (not the config file directory)
- **Changes take effect on browser refresh without recompiling** — ideal for development

## Pages

| Route | Page | Description |
|-------|------|-------------|
| `#/overview` | Overview | Message-drop trend (abnormal / non-subscriber dual tabs), connection / topic / subscription trend charts, node info cards |
| `#/overview` → Nodes tab | Nodes | Node list (OS CPU load1/5/15, memory columns) and node details (client statistics) |
| `#/overview` → Feature Support tab | Feature Support | Cluster feature consistency badge, inconsistency conflict alerts (per field), feature × node matrix |
| `#/clients` | Clients | Client search with advanced filters (incl. datetime picker), list, online/offline kick |
| `#/clients/detail` | Client Detail | Connection info + session info, current subscriptions (can unsubscribe) |
| `#/retains` | Retained Messages | `topic_filter` query, pagination (prev/next), payload preview and detail dialog |

## Internationalization

The Dashboard ships with **12 languages** (`locales/*.json`), switchable from the top-right of the UI:

| File | Language |
|------|----------|
| `zh-CN.json` / `zh-TW.json` | Simplified Chinese / Traditional Chinese |
| `en.json` | English |
| `ar.json` / `bn.json` / `de.json` / `es.json` / `fr.json` / `hi.json` / `it.json` / `pt.json` / `ru.json` | Arabic / Bengali / German / Spanish / French / Hindi / Italian / Portuguese / Russian |

## Developer Notes

- **Versioning**: static assets in `index.html` carry `?v=` query versions, and `i18n.js` has a `_localeVer` locale-cache version. Any change to JS/CSS/locale files must bump the matching version, or the browser cache will not refresh.
- **Vue 3 Composition API**: every ref/function used in a template must be explicitly returned from `setup()`, otherwise it fails at runtime (`xxx is not a function`).
- **Dead-code detection**: to tell whether a component is actually used, grep the component registry (`pageRegistry` in `app.js` + component registration) and template references — do not rely on `index.html` script tags alone.
- **Time controls**: native `datetime-local` renders "year/month/day" text in the browser language and cannot be controlled by the page i18n, so a custom `components/datetime-picker.js` (based on `Intl.DateTimeFormat`) is used instead.

## Troubleshooting

| Issue | Cause & fix |
|-------|-------------|
| Blank page / charts not rendering | The Dashboard loads Vue/ECharts from unpkg.com; offline environments must inline them or self-host a CDN |
| Changes not visible after editing | `?v=` version or `_localeVer` not bumped; bump it and hard-refresh (Ctrl+F5) |
| Changes not applied | Embedded mode requires `cargo build`; only external-directory mode supports hot reload |
