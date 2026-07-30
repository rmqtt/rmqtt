[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-auth-http

[![crates.io](https://img.shields.io/crates/v/rmqtt-auth-http.svg)](https://crates.io/crates/rmqtt-auth-http)

RMQTT 的 HTTP 认证/授权插件。将客户端认证和 ACL 检查委托给外部 HTTP API。

## 概述

向可配置的 HTTP 端点发送请求（POST/GET/PUT），携带客户端凭证。HTTP 响应决定客户端是否允许连接、发布或订阅。支持在请求参数中使用变量替换。

- **认证**：客户端连接时，插件向 `http_auth_req.url` 发送 HTTP 请求，携带客户端凭证。2xx 响应允许连接，其他响应拒绝连接。
- **ACL 检查**：客户端发布或订阅时，插件向 `http_acl_req.url` 发送 HTTP 请求，携带访问详情。响应决定操作是否允许。

## 使用

在 `Cargo.toml` 中添加依赖：

```toml
rmqtt-auth-http = "0.21"
```

在代理启动代码中注册插件：

```rust
rmqtt_auth_http::register(&scx, true, false).await?;
```

## 配置

配置文件：`rmqtt-auth-http.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `http_timeout` | String | `"5s"` | HTTP 请求超时时间 |
| `http_headers.accept` | String | `"*/*"` | Accept 请求头 |
| `http_headers.Cache-Control` | String | `"no-cache"` | Cache-Control 请求头 |
| `http_headers.User-Agent` | String | `"RMQTT/0.15.0"` | User-Agent 请求头 |
| `http_headers.Connection` | String | `"keep-alive"` | Connection 请求头 |
| `disconnect_if_pub_rejected` | Boolean | `true` | 发布被拒绝时是否断开客户端 |
| `disconnect_if_expiry` | Boolean | `false` | 过期后是否断开客户端 |
| `deny_if_error` | Boolean | `true` | HTTP 错误时返回"拒绝"；设为 `false` 则返回"忽略" |
| `http_auth_req.url` | String | `"http://127.0.0.1:9090/mqtt/auth"` | 认证请求 URL |
| `http_auth_req.method` | String | `"post"` | HTTP 方法：`post`、`get` 或 `put` |
| `http_auth_req.headers` | Table | `{ content-type = "application/x-www-form-urlencoded" }` | 请求头（支持 `application/json`） |
| `http_auth_req.params` | Table | `{ clientid = "%c", username = "%u", password = "%P", protocol = "%r" }` | 请求参数，支持变量占位符 |
| `http_acl_req.url` | String | `"http://127.0.0.1:9090/mqtt/acl"` | ACL 检查请求 URL |
| `http_acl_req.method` | String | `"post"` | HTTP 方法：`post`、`get` 或 `put` |
| `http_acl_req.params` | Table | `{ access = "%A", username = "%u", clientid = "%c", ipaddr = "%a", topic = "%t", protocol = "%r" }` | 请求参数，支持变量占位符 |

### 变量占位符

`http_auth_req.params` 支持的占位符：

| 占位符 | 描述 |
|--------|------|
| `%u` | 用户名 |
| `%c` | 客户端 ID |
| `%a` | IP 地址 |
| `%r` | 协议名称 |
| `%P` | 密码 |

`http_acl_req.params` 支持的占位符：

| 占位符 | 描述 |
|--------|------|
| `%A` | 访问类型：`1` = 订阅，`2` = 发布 |
| `%u` | 用户名 |
| `%c` | 客户端 ID |
| `%a` | IP 地址 |
| `%r` | 协议名称 |
| `%t` | 主题 |

### 认证流程

1. 客户端携带凭证连接
2. 插件向 `http_auth_req.url` 发送 HTTP 请求，携带 `http_auth_req.params` 和 `http_auth_req.headers`
3. HTTP 响应为 2xx → 认证成功；否则 → 认证失败
4. 认证失败：拒绝客户端连接
5. 请求出错时，若 `deny_if_error = true`：拒绝客户端连接；若 `deny_if_error = false`：忽略认证结果（客户端使用默认认证规则）

### ACL 流程

1. 客户端尝试发布或订阅
2. 插件向 `http_acl_req.url` 发送 HTTP 请求，携带 `http_acl_req.params`
3. HTTP 响应为 2xx → 操作允许；否则 → 操作拒绝
4. 若 `disconnect_if_pub_rejected = true`，被拒绝的发布操作将导致客户端断开

## 配置示例

```toml
http_timeout = "5s"
http_headers.content-type = "application/json"

http_auth_req.url = "http://192.168.1.100:9090/mqtt/auth"
http_auth_req.method = "post"
http_auth_req.params = { clientid = "%c", username = "%u", password = "%P" }

http_acl_req.url = "http://192.168.1.100:9090/mqtt/acl"
http_acl_req.method = "post"
http_acl_req.params = { access = "%A", username = "%u", clientid = "%c", topic = "%t" }
```

## 依赖

- `rmqtt`（feature `plugin`）
- `reqwest`（features: `rustls-tls`, `json`）

## 许可证

MIT OR Apache-2.0
