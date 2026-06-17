[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-http-api

[![crates.io](https://img.shields.io/crates/v/rmqtt-http-api.svg)](https://crates.io/crates/rmqtt-http-api)

RESTful HTTP API plugin. Provides broker management endpoints, health checks, node info, client/subscription/route listing, MQTT operations, plugin management, statistics, and Prometheus metrics.

## Overview

Uses the `salvo` HTTP framework to serve API endpoints. Starts the HTTP server during plugin construction (`new()`). Supports hot-reload: when config changes require a restart (listen address changed), the old server is shut down after the new one starts. Forwards cluster-wide queries via gRPC.

## Usage

### Build

Add the dependency in `rmqttd/Cargo.toml`:

```toml
rmqtt-http-api = "0.21"
```

Requires `rmqtt` features: `plugin`, `metrics`, `stats`, `grpc`, `shared-subscription`.

### Register

```rust
rmqtt_http_api::register(&scx, true, false).await?;
// or with explicit name:
rmqtt_http_api::register_named(&scx, "rmqtt-http-api", true, false).await?;
```

Parameters: `(scx, default_startup, immutable)`.

## Configuration

File: `rmqtt-http-api.toml` (in the plugin config directory). Loaded via `scx.plugins.read_config_default::<PluginConfig>("rmqtt-http-api")`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `max_row_limit` | `usize` | `10000` | Maximum number of rows returned by list endpoints |
| `http_laddr` | `string` | `"0.0.0.0:6060"` | HTTP server listen address |
| `http_request_log` | `bool` | `false` | Whether to print HTTP request logs |
| `http_reuseaddr` | `bool` | `true` | Enable `SO_REUSEADDR` socket option (Unix only) |
| `http_reuseport` | `bool` | `false` | Enable `SO_REUSEPORT` socket option (Unix only) |
| `http_bearer_token` | `string` | — | Bearer token for HTTP API authentication (optional) |
| `message_type` | `u8` | `99` | gRPC message type identifier for plugin communication |
| `message_expiry_interval` | `string` | `"5m"` | Default message expiration interval for publish operations |
| `metrics_sample_interval` | `string` | `"5s"` | Metrics sampling interval |
| `prometheus_metrics_cache_interval` | `string` | `"5s"` | Prometheus metrics data caching interval |

### Configuration Source

Supports the standard RMQTT plugin config chain:

1. `{plugins.dir}/rmqtt-http-api.toml` (file, optional — uses defaults if missing)
2. `rmqtt_plugin_rmqtt_http_api_*` environment variables
3. Inline config via `ServerContext::plugins_config_map_add()`

### Example

```toml
# rmqtt-http-api.toml
max_row_limit = 10_000
http_laddr = "0.0.0.0:6060"
http_request_log = false
message_expiry_interval = "5m"
prometheus_metrics_cache_interval = "5s"
```

## API Endpoints

All endpoints are prefixed with `/api/v1`.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | List all available API endpoints |
| **Brokers** | | |
| GET | `/brokers` | Return basic information of all nodes in the cluster |
| GET | `/brokers/{id}` | Return basic information of a specific node |
| **Nodes** | | |
| GET | `/nodes` | Return status of all nodes in the cluster |
| GET | `/nodes/{id}` | Return status of a specific node |
| **Health** | | |
| GET | `/health/check` | Health check for the cluster |
| GET | `/health/check/{id}` | Health check for a specific node |
| **Clients** | | |
| GET | `/clients` | Search client information from the cluster |
| GET | `/clients/{clientid}` | Get specific client information |
| DELETE | `/clients/{clientid}` | Kick client from the cluster |
| GET | `/clients/{clientid}/online` | Check if a client is online |
| GET | `/clients/offlines` | Search offline client information |
| DELETE | `/clients/offlines` | Kick offline clients from the cluster |
| **Subscriptions** | | |
| GET | `/subscriptions` | Query subscription information from the cluster |
| GET | `/subscriptions/{clientid}` | Get subscriptions for a specific client |
| **Routes** | | |
| GET | `/routes` | Return all routing information from the cluster |
| GET | `/routes/{topic}` | Get routing information for a specific topic |
| **MQTT Operations** | | |
| POST | `/mqtt/publish` | Publish an MQTT message |
| POST | `/mqtt/subscribe` | Subscribe to an MQTT topic for a session |
| POST | `/mqtt/unsubscribe` | Unsubscribe from MQTT topics |
| **Plugins** | | |
| GET | `/plugins` | Returns information of all plugins in the cluster |
| GET | `/plugins/{node}` | Returns plugin information for a specific node |
| GET | `/plugins/{node}/{plugin}` | Get a specific plugin's info |
| GET | `/plugins/{node}/{plugin}/config` | Get a plugin's configuration |
| PUT | `/plugins/{node}/{plugin}/config/reload` | Reload a plugin's configuration |
| PUT | `/plugins/{node}/{plugin}/load` | Load/start a plugin on a node |
| PUT | `/plugins/{node}/{plugin}/unload` | Unload/stop a plugin on a node |
| **Statistics** | | |
| GET | `/stats` | Returns all statistics from the cluster |
| GET | `/stats/sum` | Summarize all statistics from the cluster |
| GET | `/stats/{id}` | Returns statistics for a specific node |
| GET | `/stats/sys` | Returns all system statistics from the cluster |
| GET | `/stats/sys/sum` | Summarize all system statistics from the cluster |
| GET | `/stats/sys/{id}` | Returns system statistics for a specific node |
| **Metrics** | | |
| GET | `/metrics` | Returns all metrics from the cluster |
| GET | `/metrics/sum` | Summarize all metrics from the cluster |
| GET | `/metrics/{id}` | Returns metrics for a specific node |
| GET | `/metrics/prometheus` | Get Prometheus metrics from the cluster |
| GET | `/metrics/prometheus/sum` | Summarize Prometheus metrics |
| GET | `/metrics/prometheus/{id}` | Get Prometheus metrics for a specific node |

### Authentication

If `http_bearer_token` is configured, all API requests (except health check) require an `Authorization: Bearer <token>` header.

## Dependencies

`rmqtt` (features: `plugin`, `metrics`, `stats`, `grpc`, `shared-subscription`), `salvo`, `tokio`, `serde`, `serde_json`, `anyhow`, `base64`, `futures`

## License

MIT OR Apache-2.0
