English | [简体中文](../zh_CN/p2p-messaging.md)

# P2P Messaging

The *rmqtt-p2p-messaging* plugin provides point-to-point (P2P) messaging support.
Clients can deliver messages directly to a target client using special topic rules, without requiring the target client to explicitly subscribe.

The plugin automatically parses the `clientid` from the topic according to the configured rules and routes the message to the correct client.

---

#### Configure P2P Messaging Rules

The topic format for P2P messaging is defined by the plugin configuration. Three modes are supported:

* **prefix**: `p2p/{clientid}/{topic}` — the `p2p` identifier is at the start.
* **suffix**: `{topic}/p2p/{clientid}` — the `p2p` identifier is at the end.
* **both**: supports both prefix and suffix formats.

Where:

* `{clientid}` is the target client ID.
* `{topic}` is the actual business topic.

Examples:

* `p2p/device001/sensor/temp` → sends to client `device001` with topic `sensor/temp`
* `sensor/temp/p2p/device001` → sends to client `device001` with topic `sensor/temp`

---

#### Plugin:

```bash
rmqtt-p2p-messaging
```

---

#### Plugin configuration file:

```bash
plugins/rmqtt-p2p-messaging.toml
```

---

#### Plugin configuration options:

```bash
##--------------------------------------------------------------------
## rmqtt-p2p-messaging
##--------------------------------------------------------------------

#"prefix": `p2p/{clientid}/{topic}` — the `p2p` identifier is at the start.
#"suffix": `{topic}/p2p/{clientid}` — the `p2p` identifier is at the end.
#"both": supports both prefix and suffix formats.
mode = "prefix"
```

---

#### Enable the plugin

By default, this plugin is not enabled.
To enable it, you must add `rmqtt-p2p-messaging` to the `plugins.default_startups` section in the main configuration file `rmqtt.toml`, for example:

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
    #"rmqtt-session-storage",
    "rmqtt-p2p-messaging",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```
