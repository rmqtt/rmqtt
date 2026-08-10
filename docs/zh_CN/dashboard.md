[English](../en_US/dashboard.md)  | 简体中文

# Dashboard（Web 管理界面）

RMQTT 内置 Web 管理界面（`rmqtt-plugins/rmqtt-http-api/dashboard/`），用于查看集群状态、指标、客户端、保留消息等。它是一个**无构建步骤**的原生 SPA（`index.html` + 原生 JS + Vue3 / ECharts CDN），由 `rmqtt-http-api` 插件提供服务。

## 访问方式

### 1. 嵌入模式（默认）

Dashboard 静态资源通过 `rust-embed` 在编译时嵌入二进制，无需外部文件：

```
http://<host>:6060/dashboard/
```

修改前端代码需要重新编译（`cargo build`）才能生效。

### 2. 外部目录模式（热更新）

在 `rmqtt-http-api.toml` 中配置 `dashboard_static_dir` 指向 Dashboard 源码目录：

```toml
dashboard_static_dir = "/path/to/rmqtt-plugins/rmqtt-http-api/dashboard"
```

- 插件将 SPA 同时挂载到 `/` 与 `/dashboard/` 两个路径
- **相对路径相对于进程 cwd**（不是配置文件所在目录）
- **改文件刷新浏览器即生效，无需重新编译**——适合开发调试

## 页面功能

| 路由 | 页面 | 说明 |
|------|------|------|
| `#/overview` | 集群概览 | 消息丢弃趋势（异常丢弃 / 无订阅者丢弃双标签切换）、连接数 / 主题数 / 订阅数趋势折线图、节点信息卡片 |
| `#/overview` → 节点 Tab | 节点 | 节点列表（操作系统 CPU 负载 load1/5/15、内存列）与节点详情（客户端统计） |
| `#/overview` → 功能支持状态 Tab | 功能支持状态 | 集群功能一致性徽章、不一致冲突告警（字段级）、功能 × 节点矩阵表 |
| `#/clients` | 客户端 | 客户端搜索与高级筛选（含日期时间选择器）、列表、在线/离线踢出 |
| `#/clients/detail` | 客户端详情 | 连接信息 + 会话信息两栏、当前订阅列表（可取消订阅） |
| `#/retains` | 保留消息 | `topic_filter` 查询、分页（上一页/下一页）、payload 预览与详情弹窗 |

## 国际化

Dashboard 内置 **12 种语言**（`locales/*.json`），可在界面右上角切换：

| 文件 | 语言 |
|------|------|
| `zh-CN.json` / `zh-TW.json` | 简体中文 / 繁體中文 |
| `en.json` | English |
| `ar.json` / `bn.json` / `de.json` / `es.json` / `fr.json` / `hi.json` / `it.json` / `pt.json` / `ru.json` | 阿拉伯语 / 孟加拉语 / 德语 / 西班牙语 / 法语 / 印地语 / 意大利语 / 葡萄牙语 / 俄语 |

## 开发须知

- **版本号机制**：`index.html` 中静态资源带 `?v=` 版本号，`i18n.js` 有 `_localeVer` 语言包缓存版本。修改任何 JS/CSS/locale 文件后必须同步递增，否则浏览器缓存不生效。
- **Vue 3 组合式 API**：模板用到的所有 ref/函数必须显式加入 `setup()` 的 return，否则运行时才报错（`xxx is not a function`）。
- **死代码判断**：组件是否实际使用，需全项目 grep 组件注册表（`app.js` 的 `pageRegistry` + components 注册）与模板引用，不能只看 `index.html` 是否加载了脚本。
- **时间控件**：原生 `datetime-local` 的「年/月/日」文案跟随浏览器语言，无法被页面 i18n 控制，因此使用自研 `components/datetime-picker.js`（基于 `Intl.DateTimeFormat` 动态生成文案）。

## 常见问题

| 问题 | 原因与处理 |
|------|-----------|
| 页面白屏/图表不渲染 | Dashboard 从 unpkg.com 加载 Vue/ECharts CDN，离线环境需自行内联或自建 CDN |
| 改了代码刷新无效 | `?v=` 版本号或 `_localeVer` 未递增，浏览器命中缓存；递增后强刷（Ctrl+F5） |
| 修改未生效 | 嵌入模式下需重新 `cargo build`；外部目录模式才支持热更新 |
