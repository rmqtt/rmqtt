English | [简体中文](../zh_CN/perm-list.md)


# Access Control List (Optional)

*RMQTT* allows for pre-setting permissions for clients during the authentication phase, which is used to control publish 
and subscribe permission checks after the client logs in. Currently, both JWT authentication and HTTP authentication 
support 'permission presets,' using an access control list (ACL) as an optional extension of the authentication result. 
For example, this can be the private claim `acl` defined in the JWT or the `acl` JSON attribute returned as part of the 
HTTP authentication response. The publish and subscribe actions after the client connects will be subject to these ACL rules.

This page introduces the ACL rules for pre-setting client permissions. Authorizing clients using the ACL rules included 
in the authentication response is concise and efficient, and is typically sufficient for most use cases.

### Access Control List Format

Starting from *RMQTT 0.8.0*, support has been added for using an access control list to specify multiple permissions, 
making it closer to the semantic meaning of ACL rules and allowing for more flexibility in usage.

The ACL includes the following fields:

| Field | Required | Description                                                                                                                    |
|------------|----------|--------------------------------------------------------------------------------------------------------------------------------|
| permission | Yes      | Whether to allow the current client's operation request; optional values: allow, deny                                          |
| action     | Yes      | The operation corresponding to the rule; optional values: publish, subscribe, all                                              |
| topic      | Yes      | The topic or topic filter corresponding to the rule, supporting topic placeholders: ${username} or ${clientid}                 |
| qos        | No       | An array that specifies the applicable message QoS for the rule, such as: [0, 1]、[1, 2]、[1, 2], The default is all QoS levels. |
| retain     | NO       | A boolean value, applicable only to publish operations, specifying whether the current rule supports publishing retained messages. Optional values are `true` or `false`, with the default being to ignore this field. |


Example:
```json
{
  "exp": 1727483084,
  "username": "rmqtt_u",
  "superuser": false,  // Super users will bypass the ACL authorization check and be granted access directly (allow).
  "acl": [
    {
      // Allows the client to publish messages to the topic `foo/${clientid}`, for example, `foo/rmqtt_c`.
      "permission": "allow",
      "action": "publish",
      "topic": "foo/${clientid}"
    },
    {
      "permission": "allow",
      "action": "subscribe",
      // The `eq` prefix means that this rule applies only to the topic filter `foo/1/#`, but not to `foo/1/x` or `foo/1/y`, etc.
      "topic": "eq foo/1/#",
      // This rule matches only QoS 1 or QoS 2, but does not match QoS 0.
      "qos": [1,2]
    },
    {
      // Allows the client to subscribe to topics that match `foo/2/#` with a QoS of 1, such as `foo/2/1`, `foo/2/+`, and `foo/2/#`.
      "permission": "allow",
      "action": "subscribe",
      "topic": "foo/2/#",
      "qos": 1
    },
    {
     // Allows the client to publish messages to the topic `foo/${username}` with `retain` equal to `false` and a QoS of 0 or 1, for example, `foo/rmqtt_u`.
      "permission": "allow",
      "action": "publish",
      "topic": "foo/${username}",
      "retain": false,
      "qos": [0,1]
    },
    {
     // Disallows the client from publishing or subscribing to the topic `foo/3`, including all QoS levels and retained messages.
      "permission": "deny",
      "action": "all",
      "topic": "foo/3"
    },
    {
      // Disallows the client from publishing retained messages to the topic `foo/4`; non-retained messages are ignored by this rule.
      "permission": "deny",
      "action": "publish",
      "topic": "foo/4",
      "retain": true
    }
  ]
}

```
If none of the ACL rules are matched, they will be ignored, and the subsequent authentication chain will continue. 
If the `rmqtt-acl.toml` configuration includes the rule `["allow", "all"]`, all ignored authentications will be allowed 
by default. Conversely, you can configure the rule `["deny", "all"]` to default to rejecting all ignored authentications.