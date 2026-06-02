[English](../../en_US/development/plugin-development.md) | [**简体中文**](plugin-development.md)

# 插件开发指南

本文档介绍如何为 RMQTT 开发插件，涵盖插件生命周期、钩子系统、配置加载和最佳实践。

---

## 插件架构概述

RMQTT 插件是独立的 Rust crate，实现 `Plugin` trait。它们可以通过钩子系统拦截 Broker 事件来扩展功能，无需修改核心代码。

## 插件生命周期

```
new(scx, name) → load_config() → init() → start() → (运行态) → stop()
```

- `new()`: 构造插件实例，加载配置
- `init()`: 注册钩子处理器
- `start()`: 激活已注册的处理器
- `load_config()`: 运行时重载配置
- `stop()`: 停用处理器

## 创建插件

### 1. Cargo.toml

```toml
[package]
name = "rmqtt-my-plugin"
version.workspace = true
description = "My custom RMQTT plugin"

[dependencies]
rmqtt = { workspace = true, features = ["plugin"] }
serde = { workspace = true, features = ["derive"] }
tokio = { workspace = true, features = ["sync"] }
async-trait.workspace = true
serde_json.workspace = true
```

### 2. 插件结构

```rust
use rmqtt::context::ServerContext;
use rmqtt::plugin::{Plugin, Register};
use rmqtt::hook::{self, Type, Handler};
use serde::Deserialize;

pub struct MyPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

register!(MyPlugin::new);
```

### 3. 配置结构体

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginConfig {
    pub setting_one: String,
    pub enable_feature: bool,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            setting_one: "default".into(),
            enable_feature: true,
        }
    }
}
```

### 4. 实现 Plugin trait

```rust
#[async_trait]
impl Plugin for MyPlugin {
    async fn init(&mut self) -> Result<()> {
        self.register.add(Type::MessagePublish, handler).await;
        Ok(())
    }
    async fn start(&mut self) -> Result<()> {
        self.register.start().await
    }
    async fn stop(&mut self) -> Result<bool> {
        self.register.stop().await
    }
}
```

完整代码示例见英文版文档。

## 注册插件

1. 添加到 `rmqtt-plugins/Cargo.toml` 作为可选依赖
2. 在 `rmqtt-plugins/src/lib.rs` 添加 feature 标志和 re-export
3. 在 `rmqtt-bin/Cargo.toml` 添加依赖和 `[package.metadata.plugins]` 配置

## 配置加载

```rust
// 文件必须存在
let cfg: MyConfig = scx.plugins.load_config("my-plugin")?;

// 文件可选，缺失时使用默认值
let cfg: MyConfig = scx.plugins.load_config_default("my-plugin")?;
```

配置来源优先级：`{plugins.dir}/{name}.toml` → `rmqtt_plugin_{name}_*` 环境变量 → 内联配置

## 钩子参考

| Hook 类型 | 触发时机 | 返回值 |
|-----------|---------|--------|
| `MessagePublish` | 收到 PUBLISH | `(bool, Option<MessagePublishResult>)` |
| `ClientAuthenticate` | CONNACK 之前 | `(bool, Option<ConnAckReason>)` |
| `SubscribeCheckAcl` | 订阅 ACL 检查 | `(bool, Option<SubscribeAclResult>)` |
| `PublishCheckAcl` | 发布 ACL 检查 | `(bool, Option<PublishAclResult>)` |

全部 18 种钩子类型见架构概览文档。

## 最佳实践

1. **透传处理**：除非需要修改结果，始终返回 `(true, None)`
2. **配置默认值**：实现 `Default` 并使用 `load_config_default`
3. **线程安全**：使用 `Arc<RwLock<T>>` 管理共享状态
4. **init() 注册**：在 `init()` 中注册钩子，不要在 `new()` 中
5. **stop() 返回值**：核心插件返回 `false`（不可停止），功能插件返回 `true`

## 许可证

MIT OR Apache-2.0
