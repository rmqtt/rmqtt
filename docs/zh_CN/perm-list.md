[English](../en_US/perm-list.md)  | 简体中文

# 权限列表

*RMQTT* 允许在认证阶段为客户端预设权限，用于控制客户端登录后的发布订阅权限检查。目前，JWT 认证和 HTTP 认证支持 *“权限预设”* ，使用权限列表 (ACL) 
作为认证结果的可选扩展，例如，JWT 中定义的私有声明 acl，或作为 HTTP 认证响应的一部分返回 acl JSON 属性。客户端连接后的发布和订阅动作将会受
到这些 ACL 规则的限制。

本页面介绍了针对客户端权限预设的 ACL 规则。使用包含在认证响应中的 ACL 规则对客户端进行授权，简洁高效，且通常足以满足大多数使用场景。

### 权限列表格式

从 *RMQTT 0.8.0* 开始支持使用了权限列表来指定多条权限，更接近 ACL 规则的语义且使用更加灵活。

权限列表包含以下字段：

| 字段         | 必选 | 含义                                                      |
|------------| --------- |---------------------------------------------------------|
| permission | 是     | 是否允许当前客户端的操作请求；可选值：allow、deny                     |
| action     | 是    | 规则对应的操作；可选值: publish、subscribe、 all                     |
| topic      | 是    | 规则对应的主题或主题过滤器，支持主题占位符：${username} 或 ${clientid}         |
| qos        | 否    | 数组，指定规则适用的消息 QoS，如 [0, 1]、[1, 2]、[1, 2]，默认为全部 QoS       |
| retain     | 否    | 布尔值，仅用于发布操作，指定当前规则是否支持发布保留消息，可选值有 true、false，默认：将忽略检查此字段 |


示例：
```json
{
  "exp": 1727483084,
  "username": "rmqtt_u",
  "superuser": false,  // 超级用户将跳过ACL授权检查，并直接通过(allow)。
  "acl": [
    {
      // 允许客户端发布 foo/${clientid} 主题的消息，例如 foo/rmqtt_c
      "permission": "allow",
      "action": "publish",
      "topic": "foo/${clientid}"
    },
    {
      "permission": "allow",
      "action": "subscribe",
      // `eq` 前缀意味着该规则仅适用于主题过滤器 foo/1/#，但不适用于 foo/1/x 或 foo/1/y 等
      "topic": "eq foo/1/#",
      // 该规则只匹配 QoS 1或QoS 2 但不匹配 QoS 0
      "qos": [1,2]
    },
    {
      // 允许客户端订阅与 foo/2/# 匹配且qos为1的主题，例如 foo/2/1、foo/2/+、foo/2/#
      "permission": "allow",
      "action": "subscribe",
      "topic": "foo/2/#",
      "qos": 1
    },
    {
     // 允许客户端发布 foo/${username} 主题且retain等于false、qos为0或1的消息，例如 foo/rmqtt_u
      "permission": "allow",
      "action": "publish",
      "topic": "foo/${username}",
      "retain": false,
      "qos": [0,1]
    },
    {
     // 禁止客户端发布或订阅 foo/3 主题，包括所有 QoS 级别和保留消息
      "permission": "deny",
      "action": "all",
      "topic": "foo/3"
    },
    {
      // 禁止客户端发布 foo/4 主题的保留消息，消息为非保留消息则忽略此规则
      "permission": "deny",
      "action": "publish",
      "topic": "foo/4",
      "retain": true
    }
  ]
}

```
如果所有ACL规则都没有被匹配，将被忽略并继续执行后续认证链，如果rmqtt-acl.toml中配置了["allow", "all"]规则，默认将通过所有被忽略的认证，
可通过配置["deny", "all"]规则来默认拒绝所有被忽略的认证。

