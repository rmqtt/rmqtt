[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-auth-http

[![crates.io](https://img.shields.io/crates/v/rmqtt-auth-http.svg)](https://crates.io/crates/rmqtt-auth-http)

HTTP authentication plugin for RMQTT. Delegates client authentication and ACL checks to an external HTTP API.

## Overview

Sends HTTP requests (POST/GET/PUT) to configurable endpoints with client credentials. The HTTP response determines whether the client is allowed to connect, publish, or subscribe. Supports variable substitution in request parameters.

- **Authentication**: When a client connects, the plugin sends an HTTP request to `http_auth_req.url` with the client's credentials. A successful response (2xx) allows the connection; any other response denies it.
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
| `deny_if_error` | Boolean | `true` | Return 'Deny' on HTTP error; if false, return 'Ignore' |
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
3. If HTTP response is 2xx → authentication success; otherwise → authentication failure
4. On failure: client is denied connection
5. On error with `deny_if_error = true`: client is denied connection; with `deny_if_error = false`: the auth result is ignored (client proceeds with default auth rules)

### ACL Flow

1. Client attempts to publish or subscribe
2. Plugin sends HTTP request to `http_acl_req.url` with `http_acl_req.params`
3. If HTTP response is 2xx → operation allowed; otherwise → operation denied
4. If `disconnect_if_pub_rejected = true`, a denied publish causes client disconnection

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
