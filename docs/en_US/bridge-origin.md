English | [简体中文](../zh_CN/bridge-origin.md)

# Bridge Origin

The `rmqtt-bridge-origin` plugin is responsible for identifying whether an MQTT client originates from a bridge-ingress-mqtt or bridge-egress-mqtt connection.

When bridge plugins (such as `rmqtt-bridge-ingress-mqtt` or `rmqtt-bridge-egress-mqtt`) connect to remote MQTT brokers, they generate client IDs with identifiable markers. For example:

```bash
# Ingress bridge client ID example
node-a:ingress:node-b

# Egress bridge client ID example
node-a:egress:node-b
```

The `rmqtt-bridge-origin` plugin listens for the `ClientConnected` hook event after a client successfully connects, checks the `client_id` against configurable markers, and writes the bridge origin information into `session.extra_attrs`. This metadata can then be consumed by other plugins for purposes such as anti-loop or routing decisions.

**Note:** This plugin is only responsible for identification and metadata storage — it does not perform anti-loop, message filtering, message dropping, routing control, or bridge forwarding logic. These responsibilities belong to other bridge plugins.

#### Plugin:

```bash
rmqtt-bridge-origin
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-origin.toml
```

#### Plugin Configuration Options:

```bash
##--------------------------------------------------------------------
## rmqtt-bridge-origin
##--------------------------------------------------------------------

# Marker string to identify ingress bridge clients.
# If a client_id contains this substring, it is treated as an ingress bridge client.
# Default: ":ingress:"
#ingress_marker = ":ingress:"

# Marker string to identify egress bridge clients.
# If a client_id contains this substring, it is treated as an egress bridge client.
# Default: ":egress:"
#egress_marker = ":egress:"

# Key used to store BridgeOrigin in session.extra_attrs.
# Default: "bridge_origin"
#attr_key = "bridge_origin"
```

#### Data Model:

The bridge origin information is stored as typed data in `session.extra_attrs` under the configurable `attr_key` (defaults to `"bridge_origin"`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDirection {
    Ingress,
    Egress,
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

#### Usage in Other Plugins:

Other plugins can read the stored bridge origin data from the client session.
The attribute key used to read the data must match the `attr_key` configured in `rmqtt-bridge-origin` (defaults to `"bridge_origin"`):

```rust
if let Some(origin) = session
    .extra_attrs
    .read()
    .await
    .get::<BridgeOrigin>("bridge_origin")
{
    if origin.is_ingress() {
        log::info!("message from ingress bridge, skip forwarding");
        return (true, acc);
    }
}
```

This pattern is particularly useful in bridge egress plugins to avoid forwarding loops:
messages that arrived via an ingress bridge should not be forwarded back to a remote broker by an egress bridge.

#### Client ID Pattern Recognition:

The plugin recognizes bridge clients by matching substrings in the `client_id`. The markers are configurable, with the following defaults:

| Bridge Type | Default Marker | Example client_id |
|-------------|----------------|-------------------|
| Ingress     | `:ingress:`    | `prefix:bridge_a:ingress:node1:0:1` |
| Egress      | `:egress:`     | `prefix:bridge_b:egress:node2:0:1` |

These markers align with the auto-generated client ID patterns used by `rmqtt-bridge-ingress-mqtt` and `rmqtt-bridge-egress-mqtt` plugins.

#### Conflict Detection:

If a `client_id` contains both ingress and egress markers simultaneously (which should not occur under normal operation), the plugin logs a warning and ignores the connection to avoid ambiguity:

```bash
WARN bridge-origin: client_id contains both ingress and egress markers, client_id=...
```

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-origin` entry to the
`plugins.default_startups` configuration in the main configuration file `rmqtt.toml`, as shown below:
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

