[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-bridge-egress-reductstore

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-reductstore.svg)](https://crates.io/crates/rmqtt-bridge-egress-reductstore)

ReductStore 出站桥接插件。将 MQTT 发布消息转发到 ReductStore 时序数据库。

## 使用方式

```toml
rmqtt-bridge-egress-reductstore = "0.21"
```

```rust
rmqtt_bridge_egress_reductstore::register(&scx, true, false).await?;
```

## 配置

文件：`rmqtt-bridge-egress-reductstore.toml`

```toml
[[bridges]]
enable = true
name = "bridge_reductstore_1"
servers = "http://127.0.0.1:8383"
producer_name_prefix = "producer_1"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.bucket = "bucket1"
remote.entry = "test1"
remote.quota_size = 1_000_000_000
remote.exist_ok = true
```

| 选项 | 类型 | 默认值 | 说明 |
|--------|------|---------|------|
| `bridges[].enable` | `bool` | `true` | 启用桥接 |
| `bridges[].name` | `string` | — | 桥接名称 |
| `bridges[].servers` | `string` | — | ReductStore HTTP 地址 |
| `bridges[].entries[].local.topic_filter` | `string` | — | 本地主题过滤 |
| `bridges[].entries[].remote.bucket` | `string` | — | 存储桶名称 |
| `bridges[].entries[].remote.entry` | `string` | — | 条目键 |
| `bridges[].entries[].remote.quota_size` | `u64` | — | 存储桶配额（字节） |

## 依赖

`rmqtt`（feature `plugin`）、`reduct-rs`、`tokio`

## 许可证

MIT OR Apache-2.0
