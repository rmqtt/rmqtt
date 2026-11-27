[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-auth-http

[![crates.io](https://img.shields.io/crates/v/rmqtt-auth-http.svg)](https://crates.io/crates/rmqtt-auth-http)

HTTP authentication plugin for RMQTT. Delegates client authentication and ACL checks to an external HTTP API.

> **Note — authentication chain and ACL interaction:** This plugin participates in RMQTT's priority-ordered authentication chain and does not change or override ACL behavior. When it cannot make a decision (for example, the auth service returns a non-2xx status such as 404/500), it yields `ignore` and the chain continues with the next plugin. The built-in `rmqtt-acl` plugin is the terminal chain member, and its default final rule `["allow", "all"]` resolves to ALL operations including CONNECT — it explicitly allows connections left as `ignore`, even with `allow_anonymous = false`. If you enable this (or any custom) authentication plugin, configure your ACL rules accordingly: comment out `["allow", "all"]` and enable `["deny", "all"]` in `rmqtt-acl.toml` so only clients explicitly allowed by your authentication can connect (fail-closed). See the `rmqtt-acl` README for details.

## Overview

Sends HTTP requests (POST/GET/PUT) to configurable endpoints with client credentials. The HTTP response determines whether the client is allowed to connect, publish, or subscribe. Supports variable substitution in request parameters.

- **Authentication**: When a client connects, the plugin sends an HTTP request to `http_auth_req.url` with the client's credentials. The **2xx response body** decides the result: `allow`, `deny`, or `ignore` (plain text), or a JSON document with a `result` field. A **non-2xx** status code (404/500/502, ...) is treated as `ignore` — the auth chain continues. If the request itself fails (connection refused, timeout, DNS/TLS error), `deny_if_error = true` turns it into a `deny`.
- **ACL check**: When a client publishes or subscribes, the plugin sends an HTTP request to `http_acl_req.url` with the access details. The response determines whether the operation is allowed.

## Usage

Add the dependency to `Cargo.toml`:

```toml
rmqtt-auth-http = "0.21"
```

Register the plugin in your broker startup code:

```rust
rmqtt_auth_http::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-auth-http.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `http_timeout` | String | `"5s"` | HTTP request timeout duration |
| `http_headers.accept` | String | `"*/*"` | Accept header value |
| `http_headers.Cache-Control` | String | `"no-cache"` | Cache control header |
| `http_headers.User-Agent` | String | `"RMQTT/0.15.0"` | User agent header |
| `http_headers.Connection` | String | `"keep-alive"` | Connection header |
| `disconnect_if_pub_rejected` | Boolean | `true` | Disconnect client if publish is rejected |
| `disconnect_if_expiry` | Boolean | `false` | Disconnect client after expiry |
| `deny_if_error` | Boolean | `true` | Return 'Deny' when the HTTP request itself fails (transport-layer errors: connection refused, timeout, DNS/TLS failure). Non-2xx status responses (404/500/502, ...) always yield 'Ignore'. If `false`, transport errors also yield 'Ignore' |
| `http_auth_req.url` | String | `"http://127.0.0.1:9090/mqtt/auth"` | Authentication request URL |
| `http_auth_req.method` | String | `"post"` | HTTP method: `post`, `get`, or `put` |
| `http_auth_req.headers` | Table | `{ content-type = "application/x-www-form-urlencoded" }` | Request headers (supports `application/json`) |
| `http_auth_req.params` | Table | `{ clientid = "%c", username = "%u", password = "%P", protocol = "%r" }` | Request parameters with variable placeholders |
| `http_acl_req.url` | String | `"http://127.0.0.1:9090/mqtt/acl"` | ACL check request URL |
| `http_acl_req.method` | String | `"post"` | HTTP method: `post`, `get`, or `put` |
| `http_acl_req.params` | Table | `{ access = "%A", username = "%u", clientid = "%c", ipaddr = "%a", topic = "%t", protocol = "%r" }` | Request parameters with variable placeholders |

### Variable Placeholders

For `http_auth_req.params`:

| Placeholder | Description |
|-------------|-------------|
| `%u` | Username |
| `%c` | Client ID |
| `%a` | IP address |
| `%r` | Protocol name |
| `%P` | Password |

For `http_acl_req.params`:

| Placeholder | Description |
|-------------|-------------|
| `%A` | Access type: `1` = subscribe, `2` = publish |
| `%u` | Username |
| `%c` | Client ID |
| `%a` | IP address |
| `%r` | Protocol name |
| `%t` | Topic |

### Authentication Flow

1. Client connects with credentials
2. Plugin sends HTTP request to `http_auth_req.url` with `http_auth_req.params` and `http_auth_req.headers`
3. **2xx response**: the body decides — `allow` (connection accepted), `deny` (connection rejected), or `ignore` (the auth chain continues). For JSON responses the `result` field decides
4. **Non-2xx response** (404/500/502, ...): treated as `ignore`; the auth chain continues, **regardless of `deny_if_error`**
5. **Request-level failure** (connection refused, timeout, DNS/TLS error): `deny_if_error = true` denies the connection; `deny_if_error = false` yields `ignore` (the auth chain continues)

> **Warning**: an `ignore` result leaves the final decision to the remaining plugins in the auth chain. The built-in rmqtt-acl plugin ships a default `["allow", "all"]` rule whose omitted action column resolves to **all operations including CONNECT**, so such connections are explicitly allowed by that rule — even with `allow_anonymous = false`. When enabling a custom auth plugin, comment out `["allow", "all"]` and enable `["deny", "all"]` in `rmqtt-acl.toml` for fail-closed behavior (see the rmqtt-acl README).

### ACL Flow

1. Client attempts to publish or subscribe
2. Plugin sends HTTP request to `http_acl_req.url` with `http_acl_req.params`
3. **2xx response**: the body decides — `allow`, `deny`, or `ignore` (the ACL chain continues). For JSON responses the `result` field decides
4. **Non-2xx response**: treated as `ignore`; the ACL chain continues, regardless of `deny_if_error`
5. **Request-level failure**: `deny_if_error = true` denies the operation; `deny_if_error = false` yields `ignore`
6. If `disconnect_if_pub_rejected = true`, a denied publish causes client disconnection

## Example Configuration

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

## Dependencies

- `rmqtt` (feature `plugin`)
- `reqwest` (features: `rustls-tls`, `json`)

## License

MIT OR Apache-2.0
