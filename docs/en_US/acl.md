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
The built-in ACL has the lowest priority and can be overridden by the ACL plugin. If you want to disable it, you
can comment on all the rules. After the rules file is changed, RMQTT Broker needs to be restarted to make them take
effect. The plugin itself cannot be stopped through the plugin API (it is built in); to disable it entirely, list it
in `plugins.disabled_default_startups`.
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

After the `rmqtt-acl.toml` modification is completed, it will not be automatically loaded into the RMQTT Broker system,
but needs to be performed manually:

```bash
curl -X PUT "http://127.0.0.1:6060/api/v1/plugins/1/rmqtt-acl/config/reload"
```

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




