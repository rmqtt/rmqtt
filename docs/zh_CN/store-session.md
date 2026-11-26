[English](../en_US/store-session.md)  | 简体中文

# 存储会话信息

连接信息，订阅关系，离线消息，飞行消息将被存储。

当连接成功后存储“连接信息”，每次订阅成功后存储“订阅关系”，会话连接持续期间会定期刷新最后操作时间，会话断开连接时会存储飞行中的消息，在会话断连但未过期期间
会存储离线消息。

当RMQTT服务节点重启时会载入未过期会话基本信息和订阅关系，转发未过期离线消息和飞行消息。如果会话已经过期，将丢弃所有信息。

#### 插件：

```bash
rmqtt-session-storage
```

#### 插件配置文件：

```bash
plugins/rmqtt-session-storage.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-session-storage
##--------------------------------------------------------------------

##sled, redis, redis-cluster
storage.type = "sled"

##sled
storage.sled.path = "/var/log/rmqtt/.cache/session/{node}"
storage.sled.cache_capacity = "3G"

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "session-{node}"

##redis-cluster
storage.redis-cluster.urls = ["redis://127.0.0.1:6380/", "redis://127.0.0.1:6381/", "redis://127.0.0.1:6382/"]
storage.redis-cluster.prefix = "session-{node}"

##All circuit-breaker parameters (failure rate, window, etc.) are inherited
##from the global `[circuit_breaker]` section in `rmqtt.toml`.
##Only the backend operation timeout can be overridden here.
##
##Backend storage operation timeout. If a storage call exceeds this duration
##it is aborted and counted as a failure by the circuit breaker.
##Set to "0s" to disable.
##Default: "15s"
#backend_timeout = "15s"
```

当前支持"sled"、"redis"和"redis-cluster"三种存储引擎。"sled"是存储在本地，需要配置存储位置和在内存中的缓存容量，适当大小可以提高读写效率。
前缀配置方便不同rmqtt节点使用同一套redis存储服务。{node}将被替换为当前节点标识。

熔断器（Circuit Breaker）参数继承自主配置文件 `rmqtt.toml` 中的全局 `[circuit_breaker]` 配置段（失败率阈值、滑动窗口类型/大小、最小调用次数、OPEN 持续时间、慢调用阈值等）。插件仅允许覆盖存储后端操作超时 `backend_timeout`（默认值：`"15s"`，设为 `"0s"` 可禁用超时）。

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-session-storage”项，如：
```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "rmqtt-plugins/"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-retainer",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-sys-topic",
    #"rmqtt-message-storage",
    "rmqtt-session-storage",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

## 清空会话存储

以下场景需要把**已持久化的会话信息全部清除**，确保节点启动后不会恢复旧的订阅关系：

- 收紧或撤销 ACL 规则（见 [内置 ACL](acl.md#规则变更流程)）——持久会话的订阅会在节点启动时被重新注册进路由表，
  不清除则撤销不会生效，离线期间的消息还会在重连时补发；
- 需要让所有客户端回到“无会话”状态（`session_present = 0`）。

**必须先停止节点，再清理**：节点运行期间会话状态会被持续写回，运行中清理无效。

`sled`（默认引擎，每个节点一个目录，`{node}` 即 `node.id`，默认为 `1`）：

```bash
rm -rf /var/log/rmqtt/.cache/session/1
```

`redis` / `redis-cluster`（键名带 `storage.redis.prefix` / `storage.redis-cluster.prefix` 前缀）：

```bash
# 先抽样确认前缀与键名（前缀默认为 session-{node}，节点 1 即 session-1）
redis-cli --scan --pattern 'session-1*' | head
# 分批删除，避免长时间阻塞；不要使用 KEYS / FLUSHDB
redis-cli --scan --pattern 'session-1*' | xargs -r -n 100 redis-cli UNLINK
```

> `storage.sled.path` 与 redis 前缀均可自定义，请以实际配置为准；多节点部署需对每个节点分别执行。

清空后的影响：离线消息、飞行中消息、订阅关系与 `session_present` 全部丢失，客户端重连后处于全新会话状态，
需要重新订阅。










