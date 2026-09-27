[English](../en_US/flapping.md)  | 简体中文

# 连接抖动防护

一台不断重连的故障设备、一个失控的客户端、一次撞库攻击，从外部看是同一件事：短时间内同一来源发起大量 CONNECT。`rmqtt-flapping` 按 ClientId、用户名、来源地址三个维度在滑动窗口内统计这些尝试，一旦某个键超过阈值就被**封禁**——其后每次尝试都会被拒绝，持续一段可配置的时间，随后自动解封。

#### 为什么做成插件

判定挂在 `ClientConnect` 钩子上，**每一条**连接都会经过它，包括**不带用户名的连接**。后者根本不会进入认证链（`allow_anonymous = true` 且无用户名时，`client_authenticate` 直接返回），认证类插件既看不见也拦不住它们。`ClientConnect` 可以，因此无需改动 broker 核心即可实现连接门禁。

门禁在**认证之前**执行：被封禁的客户端只让 broker 付一次哈希查找的代价，而不是一次口令校验；携带合法凭据也无法绕过封禁。

#### 插件：

```bash
rmqtt-flapping
```

#### 插件配置文件：

```bash
plugins/rmqtt-flapping.toml
```

#### 插件配置项：

```toml
##--------------------------------------------------------------------
## rmqtt-flapping
##--------------------------------------------------------------------
## 详见 https://github.com/rmqtt/rmqtt/blob/master/docs/zh_CN/flapping.md

## 是否启用门禁。
enable = true

## *每个维度* 记录的键数量上限。满表后，本应新建键的尝试会被放行
## 但不记账，而不是被拒绝。
max_track = 100_000

## 过期窗口与到期封禁的清理周期。不得小于 1s。
gc_interval = "10s"

## 呈现某个策略表即启用该维度。下面三张表中只有 by_clientid 未注释，
## 因此该配置恰好启用一个维度。
[by_clientid]
## 统计窗口宽度。
window_time = "1m"
## 窗口内达到该次数即触发封禁；达到该次数的那一次尝试本身也被拒绝。
max_count = 15
## 封禁时长。封禁生效期间不会被延长。
ban_time = "5m"

#[by_username]
#window_time = "1m"
#max_count = 15
#ban_time = "5m"

#[by_peerhost]
#window_time = "30s"
#max_count = 100
#ban_time = "10m"

## 通知
notify_sys_topic = true
sys_topic = "$SYS/brokers/{node}/flapping/banned"
sys_topic_qos = 0
message_expiry_interval = "5m"
notify_on_every_refusal = false
notify_on_unban = true
sys_topic_unban = "$SYS/brokers/{node}/flapping/unbanned"

## 白名单（精确匹配，按维度分别检查）
allow_clientids = []
allow_usernames = []
allow_peerhosts = []
```

## 算法

### 维度

一个*维度* = 一份独立策略 + 一张独立表。每条连接按已启用的维度各检查一次，**第一个**拒绝它的维度决定最终结果。

| 维度 | 按什么记账 | 配置表 | 说明 |
|------|-----------|--------|------|
| `clientid` | ClientId | `[by_clientid]` | 空 ClientId 且 `CleanSession` / `CleanStart` 为 `1` 的连接会拿到**服务端生成的** ClientId，这类连接永远不共享键，只有 `by_peerhost` 能抓住它们。 |
| `username` | Username | `[by_username]` | 不带用户名的连接不记账。 |
| `peerhost` | 来源 IP 地址 | `[by_peerhost]` | 不含端口。这是唯一能抓住「不断更换随机 ClientId」的维度。 |

未配置的维度不参与检查。**所有**策略表都缺失时，插件会正常加载但完全不生效——它会报告自己的（空）状态，但不拒绝任何连接。

### 滑动窗口与阈值

每个键有一个宽度为 `window_time` 的窗口，以及窗口内已见尝试数的计数。

* `max_count` 是窗口内触发封禁的次数，且**达到该次数的那一次尝试本身也被拒绝**。`max_count = 4` 时，第 1、2、3 次被接受，第 4 次被拒绝并封禁该键。
* 封禁持续 `ban_time`，且**生效期间不会被延长**：持续猛打不会把解封时间往后推，封禁一到点就恢复完整的尝试额度。
* `max_count = 0` 视为配置失误，会连同一条告警一起被替换为 `1`。

### 为什么触发封禁时要丢弃窗口

如果触发封禁的窗口被保留，那么当 `ban_time` 小于 `window_time` 时，解封后的第一次尝试会看到窗口里还留着旧的尝试记录，于是**立即再次触发封禁**——封禁永远结束不了。因此创建封禁时会**丢弃该键的窗口**，解封后的额度一律从零开始。选择 `ban_time` 与 `window_time` 时请按这个语义来理解：「窗口内 N 次，然后休息 M」。

### 容量、GC 与时间序索引

窗口表与封禁表的规模由 `max_track`（**每维度**）限制，这个上界正是「随机 ClientId 洪水把表撑爆」的防线。

当一个从未见过的键到来而表已满时，门禁会先跑一次清理；若清理没有腾出空间，就**放行这次尝试但不记账**。选择拒绝会把「未知键洪水」变成对所有其它客户端的故障，因此这里采取 fail-open：攻击者用随机 ClientId 刷连接，在表饱和之后只是不再被统计。请把 `max_track` 设为你预期的**合法**键数量之上，并依靠 `by_peerhost`（键空间只是一个很小的 IP 集合）来对付这类攻击。

清理任务每 `gc_interval` 运行一次，淘汰已过期的条目。它的实现刻意做得很便宜：每个截止时间（窗口过期、封禁到期）同时被记入一个**时间序索引**（`BTreeSet<(deadline_ms, key)>`），一次清理只从索引**队首**连续弹出已到期项，遇到第一个未到期项立即返回。因此清理开销与「真正过期的条目数」成正比，而与表的规模无关；空闲 broker 每个周期只付一次读时钟的代价。`gc_interval` 只决定「内存滞留时间」与「唤醒次数」之间的取舍——热路径完全不碰索引，且索引锁与表锁不会嵌套持有。

## 拒绝码

本插件对所有拒绝使用同一个理由：**banned（已封禁）**。

| MQTT 版本 | 码值 | 名称 | 选择理由 |
|-----------|------|------|----------|
| 5.0 | `0x8A` | Banned | v5 中有语义完全对应的码。 |
| 3.1.1 / 3.1 | `0x05` | Not authorized | v3 没有「封禁」码，只能取最接近的一个。 |

对 v3 刻意**不用** `0x03`（*Server unavailable*）：它告诉客户端「服务端暂时不可用」，会诱导客户端立即重连——正是封禁想要制止的行为。`0x05` 是一个终止性的答复。

v5 码的文案由 `rmqtt-codec` 提供（`ConnectAckReason::reason()` → `"banned"`）。

## 通知

### `$SYS` 主题

每个事件一条消息，由 broker 内部的 `system` 客户端发布，因此任何插件都能通过常规的发布钩子看到（并改写）它。

| 配置项 | 默认值 | 作用 |
|--------|--------|------|
| `notify_sys_topic` | `true` | 下面两个主题的总开关。 |
| `sys_topic` | `$SYS/brokers/{node}/flapping/banned` | `{node}` 会被替换为节点 ID。 |
| `sys_topic_qos` | `0` | 默认至多一次：封禁通知不应被重试灌向本已吃力的业务系统。 |
| `message_expiry_interval` | `5m` | 在 `rmqtt-retainer` 恰好加载时由它消费。 |
| `notify_on_every_refusal` | `false` | 是否连「封禁生效期间每一次被拒的尝试」也通告，而不只是通告封禁本身。默认关闭：否则一个顽固的攻击者会用「每次重连一条消息」把该主题灌满。 |
| `notify_on_unban` | `true` | 是否通告通过管理接口手工解除的封禁。自然到期的封禁**永不**通告，主题因此不会被例行噪音填满。 |
| `sys_topic_unban` | `$SYS/brokers/{node}/flapping/unbanned` | 手工解封的通告主题。 |

**创建封禁** —— `event` 为 `"banned"`：

```json
{
  "node": 1,
  "event": "banned",
  "dimension": "clientid",
  "key": "device-01",
  "clientid": "device-01",
  "username": "sensor",
  "ipaddress": "192.168.1.10:54321",
  "listener_id": 1,
  "proto_ver": 5,
  "count": 4,
  "window_time_secs": 60,
  "ban_time_secs": 300,
  "banned_at": "2026-09-27 11:02:14.118",
  "banned_until": "2026-09-27 11:07:14.118"
}
```

`clientid`、`username`、`ipaddress` 是从触发封禁的那次尝试里复制过来的：封禁本身可能挂在三个维度中的任意一个上，而主题的读者想知道背后是谁。其中 `ipaddress` 是**带端口**的来源地址（如 `192.168.1.10:54321`），这一点与 `by_peerhost` 的键和 `allow_peerhosts` 不同——后两者只取裸地址；`listener_id` 是尝试到达的监听器数字 ID；`username` 在客户端未发送用户名时为 `null`。

**手工解封** —— `event` 为 `"unbanned"`：

```json
{
  "node": 1,
  "event": "unbanned",
  "reason": "manual",
  "dimension": "clientid",
  "key": "device-01",
  "count": 4,
  "clientid": "device-01",
  "ipaddress": "192.168.1.10:54321",
  "banned_at": "2026-09-27 11:02:14.118",
  "banned_until": "2026-09-27 11:07:14.118",
  "unbanned_at": "2026-09-27 11:03:02.005"
}
```

`reason` 恒为 `"manual"`：自然到期的封禁不做通告。

**封禁期间被拒** —— `event` 为 `"refused"`，仅在 `notify_on_every_refusal = true` 时发布：

```json
{
  "node": 1,
  "event": "refused",
  "dimension": "clientid",
  "key": "device-01",
  "clientid": "device-01",
  "username": "sensor",
  "ipaddress": "192.168.1.10:54321",
  "listener_id": 1,
  "proto_ver": 5,
  "time": "2026-09-27 11:02:31.777"
}
```

所有时间戳都按本地时间格式化为 `%Y-%m-%d %H:%M:%S%.3f`，与 broker 其它 `$SYS` 消息一致，因此可以直接按字符串比较。

### WebHook

拒绝同时会触发 `client_connack` web-hook 事件（`rmqtt-web-hook`），其报文体已经带有 `clientid`、`username`、`ipaddress`、`proto_ver` 与 `conn_ack`。在 web-hook 规则里启用 `client_connack` 即可收到拒绝事件，插件侧无需额外支持。

`conn_ack` 携带拒绝原因——v5 为 `"Banned"`，v3.1.1 为 `"Not authorized"`——与上面 `$SYS` 载荷取自同一套文案。

## 指标

| 指标 | 位置 | 含义 |
|------|------|------|
| `conn.flapping.banned` | `/api/v1/metrics`、Prometheus | **创建**的封禁累计数（每次判定计一次，不是每次被拒计一次）。 |
| `conn.flapping.refused` | `/api/v1/metrics`、Prometheus | 因封禁生效而被拒绝的连接累计数。 |
| `flapping_banned.count` | `/api/v1/stats`、Prometheus | 本节点当前**生效中**的封禁数。封禁到期它会**下降**，因此它放在 `stats` 而不是上面两个单调计数器里。 |
| `flapping_banned.max` | `/api/v1/stats`、Prometheus | `flapping_banned.count` 的历史峰值。 |

未安装任何门禁插件时，这四项恒为 0。`flapping_banned.count` 由常规的统计快照刷新，因此跟随 broker 的统计周期，而不是自己另立时钟。

## HTTP API

需要加载 `rmqtt-http-api` 插件。两个端点都**只回答收到该请求的那个节点**，不跨集群聚合——见下文「集群语义」。

| 端点 | 说明 |
|------|------|
| `GET /api/v1/flapping/banned` | 列出本节点生效中的封禁。 |
| `DELETE /api/v1/flapping/banned` | 手工解除一条封禁。 |

### 查询封禁

```
GET /api/v1/flapping/banned?dimension=clientid&key=device-01&offset=0&limit=100
```

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `dimension` | `string` | （全部） | `clientid`、`username` 或 `peerhost`。留空表示全部维度；未知名称返回 `400`，而不是被静默忽略的过滤条件。 |
| `key` | `string` | （全部） | 与封禁的键做精确匹配。 |
| `offset` | `usize` | `0` | 跳过的条目数。 |
| `limit` | `usize` | `max_row_limit` | 每页条数，会被裁剪到 `max_row_limit`。 |

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/flapping/banned?dimension=clientid"

{
  "available": true,
  "items": [
    {
      "dimension": "clientid",
      "key": "device-01",
      "count": 4,
      "banned_at": "2026-09-27 11:02:14.118",
      "banned_until": "2026-09-27 11:07:14.118",
      "remaining_ms": 282000,
      "last_clientid": "device-01",
      "last_ipaddress": "192.168.1.10:54321"
    }
  ],
  "has_more": false,
  "banned_count": 1
}
```

`count` 是触发封禁时窗口内的尝试次数；`remaining_ms` 在构造应答的那一刻测量。

`available` 表示本节点上是否装有门禁、且它此刻正在筛检。插件未加载，或已加载但未在筛检（未启用任何维度，或 `enable = false`）时为 `false`，此时其余字段如实地为空。正是这个标志把「该功能未投入使用」与「门禁在筛检、只是没人被封」区分开——只看 `banned_count` 做不到，两种情况下它都是 `0`。该状态下应答仍是 `200`：空列表本身就是合法结果，这个端点没有错误要报。`GET /api/v1/features` 在 `flapping` 下也报告同一状态，供只想问一次的调用方使用。

### 解除封禁

```
DELETE /api/v1/flapping/banned?dimension=clientid&key=device-01
```

两个参数都以**查询参数**形式传递，绝不放进路径段：键按原样精确匹配（ClientId 合法地允许包含空格或 `/`），塞进路径无法保证安全。

```bash
$ curl -i -X DELETE "http://localhost:6060/api/v1/flapping/banned?dimension=clientid&key=device-01"

{ "dimension": "clientid", "key": "device-01", "unbanned": true }
```

* `400` —— `dimension` 缺失或非法，或 `key` 缺失/为空。
* `404` —— 该键上本来就没有封禁可解。未在筛检的门禁根本不存在任何封禁，因此对任意键都回 `404`：没有东西可解，这正是该状态码本身的含义。
* `200` —— 封禁已解除；该键的下一次连接即可被接受。

当 `notify_on_unban` 开启时（默认开启），解封会在 `sys_topic_unban` 上通告。

## 白名单

```toml
allow_clientids = ["monitor-01"]
allow_usernames = ["admin"]
allow_peerhosts = ["192.168.1.10"]
```

精确匹配，按维度分别检查。白名单中的键既不记账也不被封禁，因此一个频繁重连的监控客户端不会触发门禁。`allow_peerhosts` 只写裸地址，不带端口。这些列表在配置重载时重新读取；重载不会触碰表本身，因此重载配置永远不会丢掉已生效的封禁。

## 集群语义

计数器与封禁表都是**节点本地**的。没有任何机制把 ClientId 绑定到某个节点，因此经负载均衡重连的客户端会把尝试分散到各节点，在每个节点上分别计数。这与 EMQX 同特性的行为一致，也意味着 `max_count` 必须结合集群规模来设置：

> 负载均衡后面有 `N` 个节点时，攻击者大约能发起 `N × max_count` 次尝试，才会被其中任一节点拒绝。

`$SYS` 通告同样是节点本地的——订阅 `$SYS/brokers/+/flapping/banned` 可以看到全部。HTTP API 刻意**不**做聚合：聚合结果会公布本节点并不执行的封禁，因此封禁列表按节点回答。

## 启用方式

默认**不启动**该插件。在 `rmqtt.toml` 的 `plugins.default_startups` 中加入它：

```toml
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
plugins.dir = "rmqtt-plugins/"
plugins.default_startups = [
    #"rmqtt-acl",
    #"rmqtt-retainer",
    "rmqtt-flapping",       # <- 启用
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

配置文件名与 crate 名一致（`rmqtt-flapping.toml`）。门禁在 `enable = true` **且**至少存在一张策略表时才会拒绝连接——没有维度就没有可筛检的对象——因此只加载插件而不配置任何维度是一个空操作，而不是配置错误。

## 限制

* **状态是节点本地的**，见「集群语义」。
* **封禁表仅存于内存**：broker 重启会清空所有封禁。若要让封禁跨重启存活，需要把插件状态放到集群级存储之上（尚未实现）。
* **不踢线**：插件只筛检新连接，不会断开「封禁创建时已在线」的客户端。需要时请用 `DELETE /api/v1/clients/{clientid}`。
* **满表 fail-open**：某个维度记录满 `max_track` 个键之后，新键会被放行但不记账（见「容量、GC 与时间序索引」）。
* **不影响单节点吞吐**：门禁在一条本就要走哈希的路径上，按已启用维度各做一次哈希查找。
* **可能被更早的 handler 静默绕过**：本插件在 `ClientConnect` 上取 `Priority::MAX / 2`，排在两个纯观察者（`rmqtt-counter`、`rmqtt-web-hook`，均取 `Priority::MAX`，且从不短路）之后。若另有一个优先级**高于 `MAX / 2`** 的 `ClientConnect` handler 并让它短路（返回 `proceed = false`），链会在它那里断掉，门禁**根本不会执行**，且没有任何提示。要与门禁共存，请让那个 handler 取**低于 `MAX / 2`** 的优先级。

## 许可证

MIT OR Apache-2.0
