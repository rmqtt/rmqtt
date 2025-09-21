English | [简体中文](../zh_CN/offline-message.md)


## Offline Message Support

Offline messages are messages that the broker stores while a client is disconnected, and delivers to the client when it reconnects and resumes its session.

---

### Functionality

- When a client connects with **Clean Session = false / Clean Start = false**, the broker will store messages with **QoS ≥ 1** that are published to that client while it is offline.  
- When the client reconnects and chooses to resume the session, the broker will deliver all stored offline messages.  
- If the client connects with **Clean Session = true / Clean Start = true** (creating a new session) or the previous session has expired, previously stored offline messages are cleared and will not be delivered.

---

### Storage Modes

#### 1. In-memory storage (default)

Even if the **`store-session` plugin is not enabled**, rmqtt still supports offline messages.  
- Messages are stored in memory.  
- Suitable for short disconnects and fast reconnection scenarios.  
- Offline messages in memory are lost when the broker process restarts.

#### 2. Persistent storage (via `rmqtt-session-storage` plugin)

When the **`rmqtt-session-storage` plugin** is enabled, offline messages are persisted together with the session.  
- Supported backends: local (sled), Redis, Redis-Cluster.  
- Sessions and offline messages can be recovered after broker restarts.  
- Recommended for production environments that require stronger reliability.

> Reference: [Session storage document](../zh_CN/store-session.md)

---

### Lifecycle & Behavior

| Stage | Offline message handling |
|-------|---------------------------|
| Client disconnects | If the session has not expired, the broker retains QoS ≥ 1 offline messages (in memory or persisted). |
| Message exceeds expiry interval | If `message_expiry_interval` is configured, offline messages that exceed this interval without being delivered are discarded (0 means no expiry). |
| Client reconnects and resumes session | Deliver all offline messages that have not expired. |
| Client reconnects with Clean Session = true / Clean Start = true | A new session is created; previous offline messages are cleared. |
| Session expiry | All offline messages and subscription state associated with the session are removed. |
| Broker restart | - In-memory only: offline messages are lost. <br> - With persistence (`store-session`): offline messages and sessions can be restored and still be delivered on client reconnect. |

---

### Related configuration (in `rmqtt.toml`)

Control offline-message related behavior in the main configuration file **`rmqtt.toml`**:

```toml
# Maximum length of the offline message queue; when exceeded, oldest messages are dropped
listener.tcp.external.max_mqueue_len = 1000

# Message expiry interval; "0" means never expire. Default: "5m"
listener.tcp.external.message_expiry_interval = "5m"

# Session expiry interval for inactive sessions. Default: "2h"
listener.tcp.external.session_expiry_interval = "2h"
````

* **max\_mqueue\_len**
  Controls the offline message queue length per session. When the queue is full, **the oldest messages are discarded** to make room for new incoming messages, preventing unbounded accumulation.

* **message\_expiry\_interval**
  Specifies per-message expiry time. Messages older than this interval (and not yet delivered) will be discarded. Set to `0` to disable expiry.

* **session\_expiry\_interval**
  Specifies how long an inactive session is kept. When the session expires, its subscriptions and offline messages are removed.

---

### Notes / Recommendations

* **Storage volume**: Large amounts of offline messages may consume significant memory or disk. Configure `max_mqueue_len` and expiry settings appropriately.
* **Ordering & duplicates**: MQTT allows possible duplicate deliveries (especially with QoS ≥ 1 and reconnects). rmqtt attempts to preserve order, but clients should handle idempotency.
* **Storage engine choice**: Use in-memory storage for development and short disconnect scenarios. For production, enable the `store-session` plugin with sled/Redis/Redis-Cluster depending on your availability and performance requirements.

