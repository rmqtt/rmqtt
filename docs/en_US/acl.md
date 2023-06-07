# Internal ACL

English | [简体中文](../zh_CN/acl.md)


The built-in ACL sets rules through files, which is simple and lightweight to use. It is suitable for projects with a
predictable number of rules, no change, or small change requirements.

ACL rules file:

```bash
plugins/rmqtt-acl.toml
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
The built-in ACL has the lowest priority and can be overridden by the ACL plugin. If you want to disable it, you
can comment on all the rules. After the rules file is changed, RMQTT Broker needs to be restarted to make them take
effect.
</font>
</div>



## Define ACL

The built-in ACL is the lowest priority rule table. If it is not hit after all the ACL checks are completed, the default
ACL rule is checked.

The rules file is described in Toml syntax:

```toml
rules = [
    # Allow "dashboard" users to subscribe to "$SYS/#" topics
    ["allow", { user = "dashboard" }, "subscribe", ["$SYS/#"]],
    # Allow users with IP address "127.0.0.1" to connect and publish/subscribe to topics "$SYS/#", "#"
    ["allow", { ipaddr = "127.0.0.1" }, "all", ["$SYS/#", "#"]],
    # Deny "All Users" subscribe to "$SYS/#" "#" Topics
    ["deny", "all", "subscribe", ["$SYS/#", { eq = "#" }]],
    # Allow any other clients connect and publish/subscribe operations
    ["allow", "all"]
]
```

1. The first rule allows clients with the username `dashboard` to subscribe to the topic ` $SYS/#`, which makes a
   special case for the third rule
2. The second rule allows clients with IP address `127.0.0.1` to connect and publish / subscribe to the
   topics ` $SYS/# `and `#`, which makes a special case for the third rule
3. The third rule prohibits all clients from subscribing to the topics `$SYS/#` and `#`
4. The fourth rule allows clients to connect and publish/subscribe to all topics

It can be seen that the default ACL is mainly to restrict the client's permissions on the system topic `$SYS/#` and the
all wildcard topic `#`.

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

After the `rmqtt-acl.toml` modification is completed, it will not be automatically loaded into the RMQTT Broker system,
but needs to be performed manually:

```bash
curl -X PUT "http://127.0.0.1:6066/api/v1/plugins/1/rmqtt-acl/config/reload"
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




