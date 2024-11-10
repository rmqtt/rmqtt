[English](../en_US/auto-subscription.md)  | 简体中文

# 自动订阅

自动订阅能够给 *RMQTT* 设置多个规则，在设备成功连接后按照规则为其订阅指定主题，不需要额外发起订阅。

#### 配置自动订阅规则

*RMQTT* 的自动订阅规则需要用户自行配置，用户可以自行添加多条自动订阅规则。

每条自动订阅规则的格式如下：
```bash
subscribes = [
    { topic_filter = "x/+/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 }
]
```

每条自动订阅规则由主题过滤器(topic_filter)、服务质量(qos)、No local、保留发布(retain_as_published)、保留处理(retain_handling)组成。

主题过滤器中可以使用占位符， ${clientid} 代表 客户端Id, 使用 ${username} 代表 客户端用户名。例如：foo/${clientid}/#


#### 插件：

```bash
rmqtt-auto-subscription
```

#### 插件配置文件：

```bash
plugins/rmqtt-auto-subscription.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-auto-subscription
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/auto-subscription.md

# Expressions can use ${clientid} to represent the client ID and ${username} to represent the client username.

subscribes = [
    { topic_filter = "x/+/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 },
    { topic_filter = "foo/${clientid}/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 },
    { topic_filter = "iot/${username}/#", qos = 1 }
]
```


默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-auto-subscription”项，如：
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
    "rmqtt-auto-subscription",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

