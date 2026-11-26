English | [简体中文](../zh_CN/acl.md)

# Internal ACL

The built-in ACL sets rules through files, which is simple and lightweight to use. It is suitable for projects with a
predictable number of rules, no change, or small change requirements.


#### Plugins:

```bash
rmqtt-acl
```

#### Plugin configuration file:

```bash
plugins/rmqtt-acl.toml
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
The built-in ACL has the lowest priority and can be overridden by other ACL plugins. If you want to disable it, you
can comment out all the rules. **Rule changes only take effect on the next node start: the plugin is registered as
`immutable`, so a runtime `config/reload` is rejected with `503 the plug-in is immutable`.** See "Changing ACL rules"
below for the complete procedure. The plugin itself cannot be stopped through the plugin API (it is built in); to
disable it entirely, list it in `plugins.disabled_default_startups` (as an immutable plugin it can no longer be started
through the API afterwards — change the configuration and restart instead).
</font>
</div>


## The ACL plugin in the authentication chain

`rmqtt-acl` is not limited to publish/subscribe authorization: it also hooks
`ClientAuthenticate`, so its rules take part in the CONNECT phase. Together
with the authentication plugins it forms a priority-ordered chain — the
authentication plugins (e.g. `rmqtt-auth-http`, default priority 100) run
first, and this plugin (default priority 10) is the terminal member:

- a matching `allow` rule explicitly allows the connection (the chain stops);
- a matching `deny` rule rejects it (`NotAuthorized`);
- if no rule matches at all, the connection is also rejected
  (`NotAuthorized`).

An authentication plugin that cannot make a decision yields `ignore`, and the
chain — i.e. the rules below — then decides. Beware that the default final
rule `["allow", "all"]` omits the action column, which resolves to **all
operations including CONNECT**: connections left as `ignore` are explicitly
allowed by that rule, even when `allow_anonymous = false`.

> **Fail-closed hardening:** when you enable a custom authentication plugin,
> comment out `["allow", "all"]` and enable `["deny", "all"]` as the final
> rule, so only clients explicitly allowed by your authentication can connect.
> The same note is documented from the auth side in the `rmqtt-auth-http` /
> `rmqtt-auth-jwt` docs.

ACL rules carried in authentication plugin responses (the `acl` field of an
`rmqtt-auth-http` JSON response or an `rmqtt-auth-jwt` token) are evaluated by
those plugins first; the file rules of this plugin only apply when those rules
do not produce a decision.


## Define ACL

The built-in ACL is the lowest priority rule table. If it is not hit after all the ACL checks are completed, the default
ACL rule is checked.

The rules file is described in Toml syntax:

```toml
rules = [
    # Allow "dashboard" users to subscribe to "$SYS/#" topics
    ["allow", { user = "dashboard" }, "subscribe", ["$SYS/#"]],
    # Allow client with IP address "127.0.0.1" to publish/subscribe to "$SYS/#" or "#" topics.
    ["allow", { ipaddr = "127.0.0.1" }, "pubsub", ["$SYS/#", "#"]],
    # Deny "All Users" subscribe to "$SYS/#" "#" Topics
    ["deny", "all", "subscribe", ["$SYS/#", { eq = "#" }]],
    # Allow any other clients connect and publish/subscribe operations
    #
    # NOTE: ["allow", "all"] and ["deny", "all"] are mutually exclusive final
    # rules — keep exactly ONE of them enabled, according to your deployment
    # (see the table below):
    #
    # * ["allow", "all"] — the action column is omitted, which resolves to ALL
    #   operations INCLUDING CONNECT. Use this for standalone deployments
    #   WITHOUT a custom authentication plugin: every client may connect, and
    #   publish/subscribe is allowed unless restricted by the rules above.
    #
    # * ["deny", "all"] — use this when a custom authentication plugin
    #   (rmqtt-auth-http, rmqtt-auth-jwt, ...) is enabled (fail-closed): only
    #   clients explicitly allowed by the authentication can connect, and
    #   publish/subscribe must be authorized by the ACL data the auth plugin
    #   returns (or the allow rules above). Connections the auth plugin leaves
    #   undecided ('ignore', e.g. a 404/500 from the auth service) are rejected.
    ["allow", "all"]
    #["deny", "all"]
]
```

1. The first rule allows clients with the username `dashboard` to subscribe to the topic ` $SYS/#`, which makes a
   special case for the third rule
2. The second rule allows clients with IP address `127.0.0.1` to publish / subscribe to the
   topics ` $SYS/# ` or `#`, which makes a special case for the third rule
3. The third rule prohibits all clients from subscribing to the topics `$SYS/#` and `#`
4. The fourth rule allows clients to connect and publish/subscribe to all topics

It can be seen that the default ACL is mainly to restrict the client's permissions on the system topic `$SYS/#` and the
all wildcard topic `#`.

### `["allow", "all"]` vs `["deny", "all"]` — which one to use

The two special final rules are mutually exclusive: keep exactly one of them enabled, according to your deployment.

| Final rule | When to use | Effect |
|------------|-------------|--------|
| `["allow", "all"]` (default) | Standalone deployments **without** a custom authentication plugin | Every client may connect; publish/subscribe is allowed unless restricted by the earlier rules |
| `["deny", "all"]` | A custom authentication plugin (`rmqtt-auth-http`, `rmqtt-auth-jwt`, ...) is **enabled** | Fail-closed: only clients explicitly allowed by the authentication can connect, and publish/subscribe must be authorized by the ACL data the auth plugin returns (or the allow rules above). Connections the auth plugin leaves undecided (`ignore`, e.g. a 404/500 from the auth service) are rejected |

Note that `["allow", "all"]` also covers CONNECT (the omitted action column
resolves to all operations), which is why it can promote an auth plugin's
`ignore` into a successful connection — see
["The ACL plugin in the authentication chain"](#the-acl-plugin-in-the-authentication-chain) above.

## rmqtt-acl.toml Writing rules

The rules in the `rmqtt-acl.toml` file are matched from top to bottom in writing order.

- Line comments are expressed as `#`.
- Each rule consists of four tuples.
- The first position of the tuple indicates that after the rule is successfully hit, the permission control operation is
  performed. The possible values are:
    * `allow`
    * `deny`
- The second position of the tuple indicates the user to which the rule takes effect. The format that can be used is:
    * `{ user = "dashboard" }`: The rule only takes effect for users whose Username is dashboard
    * `{ user = "dashboard", password = "123456", superuser = true }`：Indicates that the rule is effective for users
      with * Username * as "dashboard" and * Password * as "123456"; Superuser indicates that this user is a superuser
      and will skip authentication when publish/subscribe to messages.
    * `{ clientid = "dashboard" }`: The rule only takes effect for users whose ClientId is dashboard
    * `{ ipaddr = "127.0.0.1" }`: The rule only takes effect for users whose Source Address is "127.0.0.1"
    * `{ protocol = 4 }`: The rule only takes effect for users of MQTT protocol version 4 (3.1.1). MQTT protocol version: 3 = 3.1, 4 = 3.1.1, or 5 = 5.0
    * `{ user = "dashboard", protocol = 4 }`: The rule only takes effect for users with username "dashboard" and MQTT protocol version 4 (3.1.1)
    * `all`: The rule takes effect for all users
- The third position of the tuple indicates the operation controlled by the rule with the possible value:
    * `connect`：The rule applies to CONNECT operations
    * `publish`: The rule applies to PUBLISH operations
    * `subscribe`: The rule applies to SUBSCRIBE operations
    * `pubsub`: The rule applies to both PUBLISH and SUBSCRIBE operations
    * `all`：The rule applies to all operations (default)
- The fourth position of the tuple means the list of topics restricted by the rule. The content is given in the form of
  an array. For example:
    * `"$SYS/#"`:  **Topic Filter** which means that the rule is applied to topics that match `$SYS/#`; for example
      rules created for "$SYS/#" applies to publish/subscribe actions on topic "$SYS/a/b/c", and subscribe actions on
      topic "$SYS/#"
    * `{ eq = "#" }`: It indicates full equivalence of characters. The rule is only applied for topic `#` but not
      for `/a/b/c`, etc.
- In addition, there are two special rules:
    - `{allow, all}`: Allow all operations
    - `{deny, all}`: Deny all operations

Rule matching details:

- Rules are evaluated top to bottom. A rule takes effect only when **both** the
  user condition and the topic condition match; if the user condition matches
  but no topic does, evaluation continues with the next rule. (Rules that apply
  to CONNECT have no topic condition, so the user condition alone decides.)
- `password` is only compared for `allow` rules. A `deny` rule matches by
  username only and ignores any configured `password`.

## Changing ACL rules

`rmqtt-acl` is registered as an **immutable** plugin: `PUT /api/v1/plugins/{node}/rmqtt-acl/config/reload` returns
`503 the plug-in is immutable`. This is deliberate. Authorization is evaluated only at **CONNECT / SUBSCRIBE /
PUBLISH**; the **delivery path never checks the ACL again**, so a successful reload does not mean the revocation took
effect:

- Subscriptions that are already established are not revoked — the client keeps receiving messages.
- With `rmqtt-session-storage` enabled, persisted subscriptions are re-registered into the routing table when the node
  starts, and messages queued for the offline session are replayed when the client reconnects.

Changing ACL rules (especially **tightening or revoking** a grant) must therefore follow this procedure, otherwise you
end up with "looks revoked, still receiving":

1. **Stop every node.** In a multi-node deployment do this node by node, and make sure all of them are stopped before
   the next step — a running node writes the session state back.
2. **Clear the session store** (`rmqtt-session-storage`). Commands and impact are documented in
   [Store session information](store-session.md#clearing-the-session-store); skip if the plugin is not enabled.
3. Edit `rmqtt-acl.toml`.
4. **Start the nodes.** On reconnect each client re-issues SUBSCRIBE, which is evaluated against the new rules, and
   unauthorized subscriptions are refused (SUBACK failure, `0x80` on v3.1.1).

> **Cost**: clearing the session store drops offline messages, inflight messages and the `session_present` flag for
> **all** clients, and clients must subscribe again. To revoke a single client instead, use
> `DELETE /api/v1/clients/{clientid}` (**once per node** — it acts on the local node only): it disconnects the client
> and clears its subscriptions. A `404` means the session is not on that node, in which case use the session-store
> wipe.

> **Note**: placeholder rules (`%u` / `%c`) are expanded from the client identity at CONNECT time, so they are also
> rebuilt against the new rules only after a node restart.

## Will Messages

A Will Message is a PUBLISH the broker sends on a client's behalf when the connection drops without a
DISCONNECT. It is checked with the same publish rules as a normal PUBLISH, at the moment the broker
publishes it. A Will on a topic the client is not allowed to publish to is dropped and a `message_dropped`
event is raised. The client is not disconnected, because it is already gone.

## Placeholders

The built-in `rmqtt-acl.toml` supports only the following placeholders in the subject's field (the 4th position of the
tuple).

- `%c`: For Client ID, which is replaced by the client ID when the rule takes effect.
- `%u`: For username, which is replaced by the client's username when the rule takes effect.

E.g:

```
["allow", "all", "pubsub", ["sensor/%c/ctrl"]]
```

This means that a client with ID 'light' is **Allowed** to **Subscribe and Publish** to the `sensor/light/ctrl` topic.

::: tip Only a few simple and general rules are contained in `rmqtt-acl.toml` that make it a system-based ACL principle.
If you need to support complex, large amounts of ACL content, you should implement it in an authentication plugin.

:::
<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
Only a few simple and general rules are contained in `rmqtt-acl.toml` that make it a system-based ACL principle.
If you need to support complex, large amounts of ACL content, you should implement it in an authentication plugin.
</font>
</div>




