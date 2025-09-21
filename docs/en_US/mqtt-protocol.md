English | [简体中文](../zh_CN/mqtt-protocol.md)

# MQTT Protocol Documentation

## 1. Protocol Versions and Official Documentation

RMQTT supports the following MQTT protocol versions and strictly follows the official specifications to ensure standardized message exchange between clients and the server while maintaining cross-version compatibility:

| Version    | Official English Documentation                                                                                | Chinese Documentation                           |
| ---------- | ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| MQTT 3.1   | [MQTT V3.1 Protocol Specification](https://public.dhe.ibm.com/software/dw/webservices/ws-mqtt/mqtt-v3r1.html) | [MQTT 3.1 中文版](https://mqtt.p2hp.com/mqtt311)   |
| MQTT 3.1.1 | [MQTT Version 3.1.1](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html)                     | [MQTT 3.1.1 中文版](https://mqtt.p2hp.com/mqtt311) |
| MQTT 5.0   | [MQTT Version 5.0](https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.html)                                 | [MQTT 5.0 中文版](https://mqtt.p2hp.com/mqtt-5-0)  |

---

## 2. Connect Packet (CONNECT)

* Clients initiate a connection with the server using the **CONNECT** packet.
* Key fields include:

    * `Client Identifier`: unique client ID
    * `Username` / `Password` (optional)
    * `Keep Alive`: heartbeat interval used by the server to check client activity
    * `Clean Session` / `Clean Start` (MQTT 5.0)
    * `Will Message` (optional last will message)
    * MQTT 5.0 optional fields: `Authentication Method`, `Authentication Data`

The server responds with a **CONNACK** packet, indicating the connection result, session status, and optional protocol-level error codes (Reason Codes).

---

## 3. Subscribe and Unsubscribe

### 3.1 Subscribe (SUBSCRIBE)

* Clients subscribe to topics to receive messages.

* Packet fields include:

    * List of topic filters
    * QoS level (0, 1, 2)
    * MQTT 5.0 may include User Properties

* The server responds with **SUBACK**, confirming the subscription result and return codes for each topic.

* Supports **Shared Subscriptions**, allowing multiple clients to consume messages from the same topic collaboratively.

### 3.2 Unsubscribe (UNSUBSCRIBE)

* Clients can unsubscribe from one or more topics.
* The server responds with **UNSUBACK**, confirming the unsubscription.

---

## 4. Publish Messages (PUBLISH)

* Clients or the server send messages to a topic.
* Key fields:

    * Topic Name
    * Payload
    * QoS (0 / 1 / 2)
    * Retain flag (whether the message should be retained)
    * MQTT 5.0 optional fields: `Message Expiry Interval`, `Content Type`, `User Properties`, `Response Topic`

### 4.1 QoS Levels

| QoS | Description   |
| --- | ------------- |
| 0   | At most once  |
| 1   | At least once |
| 2   | Exactly once  |

* QoS 1 and 2 involve acknowledgment flows (PUBACK, PUBREC / PUBREL / PUBCOMP).
* Retained messages: the server stores the last message with the Retain flag; new subscribers receive it immediately.

### 4.2 Offline Messages and Expiry

* RMQTT supports **offline message queues**: QoS 1/2 messages are stored when clients are offline.
* `Message Expiry Interval` controls the message lifetime in the queue.
* Session queue length is configurable; when exceeded, **old messages are discarded** to prevent unbounded accumulation.

---

## 5. Session and State

* **Clean Session / Clean Start** determines session persistence.
* **Persistent Session**: subscription info and undelivered QoS 1/2 messages are retained.
* **Non-Persistent Session**: the server does not retain any messages after client disconnect.
* **Will Message**: published by the server if the client disconnects unexpectedly.
* **Session Reconnection**: persistent session state, including offline messages, can be restored when the client reconnects.

---

## 6. Retained and Will Messages

* **Retained Messages**: the server stores the last message of a topic; new subscribers receive it immediately.
* **Will Messages**: sent by the server when a client disconnects unexpectedly.
* Note: only the most recent retained message is stored; older ones are overwritten.

---

## 7. Topics and Wildcards

* Topic levels are separated by `/`, e.g., `home/kitchen/temperature`.

* Wildcards:

    * `+`: single-level wildcard, matches one topic level
    * `#`: multi-level wildcard, matches the current and all sub-levels

* Shared subscription topic format: `$share/<group>/<topic>`

---

## 8. MQTT 5.0 New Features

* **User Properties**: custom metadata
* **Topic Alias**: reduce packet size
* **Message Expiry Interval**
* **Response Information**
* **Server Reference**
* **Maximum Packet Size**
* **Request/Response Pattern**
* **Shared Subscription**
* **Enhanced Reason Codes**: finer-grained operation feedback

---

## 9. Packet Overview

| Packet Type | Description                                     |
| ----------- | ----------------------------------------------- |
| CONNECT     | Establish a connection                          |
| CONNACK     | Connection acknowledgment                       |
| PUBLISH     | Publish message                                 |
| PUBACK      | QoS 1 message acknowledgment                    |
| PUBREC      | QoS 2 message received                          |
| PUBREL      | QoS 2 message release                           |
| PUBCOMP     | QoS 2 message complete                          |
| SUBSCRIBE   | Subscribe to topics                             |
| SUBACK      | Subscription acknowledgment                     |
| UNSUBSCRIBE | Unsubscribe from topics                         |
| UNSUBACK    | Unsubscription acknowledgment                   |
| PINGREQ     | Heartbeat request                               |
| PINGRESP    | Heartbeat response                              |
| DISCONNECT  | Client disconnect                               |
| AUTH        | MQTT 5.0 Authentication/Authorization extension |

---

## 10. Error Handling and Reconnection Strategy

* RMQTT provides detailed **error codes** for connection exceptions.
* Clients can use returned codes to **reconnect or adjust parameters**.
* Supports **automatic reconnection**, with configurable retry intervals and maximum attempts.
* Subscription or publish failures return corresponding **Reason Codes**.

---
