[English](../en_US/mqtt-protocol.md)  | 简体中文

# MQTT 协议文档

## 1. 协议版本与官方文档

RMQTT 支持以下 MQTT 协议版本，并严格遵循官方规范，确保客户端与服务器之间的消息交换符合标准，同时保证跨版本兼容性：

| 版本         | 官方英文版                                                                                                         | 中文版                                             |
| ---------- | ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| MQTT 3.1   | [MQTT V3.1 Protocol Specification](https://public.dhe.ibm.com/software/dw/webservices/ws-mqtt/mqtt-v3r1.html) | [MQTT 3.1 中文版](https://mqtt.p2hp.com/mqtt311)   |
| MQTT 3.1.1 | [MQTT Version 3.1.1](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html)                     | [MQTT 3.1.1 中文版](https://mqtt.p2hp.com/mqtt311) |
| MQTT 5.0   | [MQTT Version 5.0](https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.html)                                 | [MQTT 5.0 中文版](https://mqtt.p2hp.com/mqtt-5-0)  |

---

## 2. 连接报文（CONNECT）

* 客户端通过 **CONNECT** 报文向服务器发起连接请求。
* 报文包含以下关键字段：

    * `Client Identifier`：客户端唯一标识
    * `Username` / `Password`（可选）
    * `Keep Alive`：心跳间隔，服务器根据此间隔检查客户端活跃状态
    * `Clean Session` / `Clean Start`（MQTT 5.0）
    * `Will Message`（可选遗嘱消息）
    * MQTT 5.0 可选字段：`Authentication Method`、`Authentication Data`

服务器响应 **CONNACK** 报文，返回连接结果、会话状态以及可选的协议级错误码（Reason Code）。

---

## 3. 订阅与取消订阅

### 3.1 订阅（SUBSCRIBE）

* 客户端订阅主题以接收消息。

* 报文包含：

    * 主题列表（Topic Filter）
    * QoS 等级（0、1、2）
    * MQTT 5.0 可包含用户属性（User Properties）

* 服务器返回 **SUBACK** 报文，确认订阅结果，包括每个主题的订阅状态码。

* 支持 **共享订阅（Shared Subscription）**，多个客户端可共同消费同一主题消息。

### 3.2 取消订阅（UNSUBSCRIBE）

* 客户端可取消对一个或多个主题的订阅。
* 服务器返回 **UNSUBACK** 报文确认操作结果。

---

## 4. 发布消息（PUBLISH）

* 客户端或服务器发送消息到主题。
* 关键字段：

    * 主题（Topic Name）
    * 消息负载（Payload）
    * QoS（0 / 1 / 2）
    * Retain 标志（是否保留消息）
    * MQTT 5.0 可包含消息过期时间（Message Expiry Interval）、内容类型（Content Type）、用户属性（User Properties）、响应主题（Response Topic）

### 4.1 QoS 说明

| QoS | 描述                  |
| --- | ------------------- |
| 0   | 最多一次（At most once）  |
| 1   | 至少一次（At least once） |
| 2   | 仅一次（Exactly once）   |

* QoS 1 和 2 会有相应的确认流程（PUBACK、PUBREC / PUBREL / PUBCOMP）
* 保留消息（Retained Message）：服务器保存最后一条带 Retain 标志的消息，新订阅者可立即接收

### 4.2 离线消息与过期

* RMQTT 支持 **离线消息队列**：当客户端离线时，服务器可保存 QoS 1/2 消息
* 消息过期时间（Message Expiry Interval）控制消息在队列中的存活时长
* 会话队列长度可配置，超过长度时，旧消息将被丢弃以防无限积压

---

## 5. 会话与状态

* **Clean Session / Clean Start** 控制会话持久化与否
* **持久会话**：保存订阅信息与未发送的 QoS 1/2 消息
* **非持久会话**：客户端断开后，服务器不保存任何消息
* **遗嘱消息（Will Message）**：客户端异常断开时，由服务器发布指定消息
* **会话重连**：客户端重连时，可恢复持久会话状态，包括未收到的离线消息

---

## 6. 保留消息与遗嘱消息

* **保留消息（Retained Messages）**：服务器保存最后一条消息，新订阅者可立即收到
* **遗嘱消息（Will Message）**：客户端异常断开时，服务器发布指定消息
* **注意**：保留消息仅保存最后一条，旧消息会被覆盖

---

## 7. 主题与通配符

* 主题层级以 `/` 分隔，如 `home/kitchen/temperature`

* 通配符：

    * `+`：单层通配符，匹配一层主题
    * `#`：多层通配符，匹配该层及子层主题

* 支持共享订阅主题格式 `$share/<group>/<topic>`

---

## 8. MQTT 5.0 新特性

* **用户属性（User Properties）**：自定义元数据
* **主题别名（Topic Alias）**：减少报文大小
* **消息过期时间（Message Expiry Interval）**
* **响应信息（Response Information）**
* **服务器参考（Server Reference）**
* **最大消息大小（Maximum Packet Size）**
* **请求响应模式（Request/Response Pattern）**
* **共享订阅（Shared Subscription）**
* **增强错误码与原因码（Reason Codes）**：更精细的操作反馈

---

## 9. 报文格式概览

| 报文类型        | 描述               |
| ----------- | ---------------- |
| CONNECT     | 建立客户端与服务器连接      |
| CONNACK     | 连接确认             |
| PUBLISH     | 发布消息             |
| PUBACK      | QoS 1 消息确认       |
| PUBREC      | QoS 2 消息接收确认     |
| PUBREL      | QoS 2 消息释放       |
| PUBCOMP     | QoS 2 消息完成确认     |
| SUBSCRIBE   | 订阅主题             |
| SUBACK      | 订阅确认             |
| UNSUBSCRIBE | 取消订阅             |
| UNSUBACK    | 取消订阅确认           |
| PINGREQ     | 心跳请求             |
| PINGRESP    | 心跳响应             |
| DISCONNECT  | 客户端断开连接          |
| AUTH        | MQTT 5.0 授权/认证扩展 |

---

## 10. 错误处理与重连策略

* RMQTT 对连接异常提供详细 **错误码**
* 客户端可根据返回码选择 **重连或调整参数**
* 支持 **自动重连机制**，可配置重试间隔和最大次数
* 订阅或发布失败可返回相应 **Reason Code**

---

