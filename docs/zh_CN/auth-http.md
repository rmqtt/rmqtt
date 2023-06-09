[English](../en_US/auth-http.md)  | 简体中文

# HTTP 认证

HTTP 认证使用外部自建 HTTP 应用认证数据源，根据 HTTP API 返回的数据判定认证结果，能够实现复杂的认证鉴权逻辑。


插件：

```bash
rmqtt-auth-http
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">rmqtt-auth-http 插件同时包含 ACL 功能，可通过注释禁用。</font>
</div>

## 认证原理

RMQTT 在设备连接事件中使用当前客户端相关信息作为参数，向用户自定义的认证服务发起请求查询权限，通过返回的 HTTP 响应信息来处理认证请求。

- 认证成功：
  - API 返回 2xx 状态码且消息体为:allow
- 认证失败：
  - API 返回 2xx 状态码且消息体为:deny
  - API HTTP 请求失败，且deny_if_error配置等于:true
- 忽略认证：
  - API 返回 2xx 状态码且消息体为:ignore, 继续执行认证链。
  - API 返回 4xx/5xx 状态码将忽略消息体并判定结果为:ignore, 继续执行认证链。
  - API HTTP 请求失败，且deny_if_error配置等于:false, 判定结果为:ignore, 继续执行认证链。
- 超级用户：
  - 认证成功 且 响应头返回“X-Superuser: true”, 超级用户将跳过ACL授权。
  
响应示例：
```json
HTTP/1.1 200 OK
X-Superuser: true
Content-Type: text/plain
Content-Length: 5
Date: Wed, 07 Jun 2023 01:29:23 GMT

allow
```

## 认证请求

进行身份认证时，RMQTT 将使用当前客户端信息填充并发起用户配置的认证查询请求，查询出该客户端在 HTTP 服务器端的认证数据。

```bash
# etc/plugins/rmqtt-auth-http.toml

## 请求地址
http_auth_req.url = "http://127.0.0.1:9090/mqtt/auth"

## HTTP 请求方法
## Value: post | get | put
http_auth_req.method = "post"

## 认证请求的 HTTP 请求头部，默认情况下配置 Content-Type 头部。
## Content-Type 头部目前支持以下值：application/x-www-form-urlencoded，application/json
http_auth_req.headers = { content-type = "application/x-www-form-urlencoded" }

## 请求参数
http_auth_req.params = { clientid = "%c", username = "%u", password = "%P" }
```

HTTP 请求方法为 GET 时，请求参数将以 URL 查询字符串的形式传递；POST、PUT 请求则将请求参数以普通表单形式或者以 Json 形式提交（由 content-type 的值决定）。

你可以在认证请求中使用以下占位符，请求时 RMQTT 将自动填充为客户端信息：

- %u：用户名
- %c：Client ID
- %a：客户端 IP 地址
- %r：客户端接入协议
- %P：明文密码

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
推荐使用 POST 与 PUT 方法，使用 GET 方法时明文密码可能会随 URL 被记录到传输过程中的服务器日志中。
</font>
</div>


# HTTP ACL

HTTP 认证使用外部自建 HTTP 应用认证授权数据源，根据 HTTP API 返回的数据判定授权结果，能够实现复杂的 ACL 校验逻辑。

插件：

```bash
rmqtt-auth-http
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
rmqtt-auth-http 插件同时包含认证功能，可通过注释禁用。
</font>
</div>


要启用 HTTP ACL，需要在 `etc/plugins/rmqtt-auth-http.toml` 中配置以下内容：

## ACL 授权原理

RMQTT 在设备发布、订阅事件中使用当前客户端相关信息作为参数，向用户自定义的认证服务发起请求权限，通过返回的 HTTP 响应信息来处理 ACL 授权请求。

- 授权成功：
  - API 返回 2xx 状态码且消息体为:allow
- 无权限：
  - API 返回 2xx 状态码且消息体为:deny
  - API HTTP 请求失败，且deny_if_error配置等于:true
- 忽略授权：
  - API 返回 2xx 状态码且消息体为:ignore, 继续执行授权认证链。
  - API 返回 4xx/5xx 状态码将忽略消息体并判定结果为:ignore, 继续执行授权认证链。
  - API HTTP 请求失败，且deny_if_error配置等于:false, 判定结果为:ignore, 继续执行授权认证链。
- 缓存授权结果：
  - 响应头返回“X-Cache: -1” 表示结果被缓存，值为缓存超时时间，单位毫秒，-1表示连接活跃期间有效。

进行发布、订阅认证时，RMQTT 将使用当前客户端信息填充并发起用户配置的 ACL 授权查询请求，查询出该客户端在 HTTP 服务器端的授权数据。

如果是超级用户，将跳过ACL授权认证。

## ACL 授权查询请求

```bash
# etc/plugins/rmqtt-auth-http.toml

## 请求地址
http_acl_req.url = "http://127.0.0.1:9090/mqtt/acl"

## HTTP 请求方法
## Value: post | get | put
http_acl_req.method = "get"

## 请求参数
http_acl_req.params = { access = "%A", username = "%u", clientid = "%c", ipaddr = "%a", topic = "%t" }

```

## 请求说明

HTTP 请求方法为 GET 时，请求参数将以 URL 查询字符串的形式传递；POST、PUT 请求则将请求参数以普通表单形式提交（content-type 为 x-www-form-urlencoded）。

你可以在认证请求中使用以下占位符，请求时 RMQTT 将自动填充为客户端信息：

- %A：操作类型，'1' 订阅；'2' 发布
- %u：客户端用户名
- %c：Client ID
- %a：客户端 IP 地址
- %r：客户端接入协议
- %t：主题

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
推荐使用 POST 与 PUT 方法，使用 GET 方法时明文密码可能会随 URL 被记录到传输过程中的服务器日志中。
</font>
</div>


# HTTP 基础请求信息

HTTP API 基础请求信息，请求头。

```bash
# etc/plugins/rmqtt-auth-http.toml

## 请求头设置
http_timeout = "5s"
http_headers.accept = "*/*"
http_headers.Cache-Control = "no-cache"
http_headers.User-Agent = "RMQTT/0.2.11"
http_headers.Connection = "keep-alive"

# 如果发布消息被拒绝，则断开连接
disconnect_if_pub_rejected = true

# 如果http请求错误，则返回“拒绝”，否则返回“忽略”
deny_if_error = true

```