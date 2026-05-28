[English](../en_US/bridge-origin.md) | 简体中文

# 桥接来源识别

`rmqtt-bridge-origin` 插件负责识别 MQTT 客户端是否来自 `rmqtt-bridge-ingress-mqtt` 或 `rmqtt-bridge-egress-mqtt` 插件建立的桥接连接。

当桥接插件（如 `rmqtt-bridge-ingress-mqtt` 或 `rmqtt-bridge-egress-mqtt`）连接到远程 MQTT 代理时，它们会生成带有可识别标记的客户端ID。例如：

```bash
# 入口桥接客户端 ID 示例
node-a:ingress:node-b

# 出口桥接客户端 ID 示例
node-a:egress:node-b
```

`rmqtt-bridge-origin` 插件监听客户端连接成功后的 `ClientConnected` 事件，检查 `client_id` 中是否包含可配置的标记，并将桥接来源信息写入 `session.extra_attrs`。其他插件可以读取这些元数据，用于防环或路由决策等场景。

**注意：** 此插件仅负责识别和元数据存储——它不执行防环、消息过滤、消息丢弃、路由控制或桥接转发逻辑。这些职责属于其他桥接插件。

#### 插件：

```bash
rmqtt-bridge-origin
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-origin.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-bridge-origin
##--------------------------------------------------------------------

# 识别入口桥接客户端的标记字符串。
# 如果 client_id 包含此子字符串，则视为入口桥接客户端。
# 默认值: ":ingress:"
#ingress_marker = ":ingress:"

# 识别出口桥接客户端的标记字符串。
# 如果 client_id 包含此子字符串，则视为出口桥接客户端。
# 默认值: ":egress:"
#egress_marker = ":egress:"

# 用于在 session.extra_attrs 中存储 BridgeOrigin 的键名。
# 默认值: "bridge_origin"
#attr_key = "bridge_origin"
```

#### 数据模型：

桥接来源信息以类型化数据存储在 `session.extra_attrs` 中，键名由 `attr_key` 配置项决定（默认为 `"bridge_origin"`）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDirection {
    Ingress,  // 入口桥接
    Egress,   // 出口桥接
}

#[derive(Debug, Clone, Copy)]
pub struct BridgeOrigin {
    pub direction: BridgeDirection,
}

impl BridgeOrigin {
    pub fn is_ingress(&self) -> bool { ... }
    pub fn is_egress(&self) -> bool { ... }
}
```

#### 在其他插件中使用：

其他插件可以从客户端会话中读取存储的桥接来源数据。
读取时使用的键名必须与 `rmqtt-bridge-origin` 中配置的 `attr_key` 一致（默认为 `"bridge_origin"`）：

```rust
if let Some(origin) = session
    .extra_attrs
    .read()
    .await
    .get::<BridgeOrigin>("bridge_origin")
{
    if origin.is_ingress() {
        log::info!("消息来自入口桥接，跳过转发");
        return (true, acc);
    }
}
```

此模式在桥接出口插件中特别有用，用于避免转发循环：
通过入口桥接到达的消息不应再由出口桥接转发回远程代理。

#### 客户端 ID 模式识别：

插件通过匹配 `client_id` 中的子字符串来识别桥接客户端。标记可通过配置自定义，默认值如下：

| 桥接类型 | 默认标记 | client_id 示例 |
|---------|---------|----------------|
| Ingress (入口) | `:ingress:` | `prefix:bridge_a:ingress:node1:0:1` |
| Egress (出口) | `:egress:`  | `prefix:bridge_b:egress:node2:0:1` |

这些标记与 `rmqtt-bridge-ingress-mqtt` 和 `rmqtt-bridge-egress-mqtt` 插件使用的自动生成客户端 ID 模式保持一致。

#### 冲突检测：

如果 `client_id` 同时包含入口和出口标记（在正常情况下不应发生），插件会记录警告日志并忽略该连接以避免歧义：

```bash
WARN bridge-origin: client_id contains both ingress and egress markers, client_id=...
```

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件 `rmqtt.toml` 中的 `plugins.default_startups` 配置中添加 `rmqtt-bridge-origin` 项，如：
```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "rmqtt-plugins/"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-plugin-template",
    #"rmqtt-retainer",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-sys-topic",
    #"rmqtt-message-storage",
    #"rmqtt-session-storage",
    #"rmqtt-bridge-ingress-mqtt",
    #"rmqtt-bridge-egress-mqtt",
    "rmqtt-bridge-origin",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

