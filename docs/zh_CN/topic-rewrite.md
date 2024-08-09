[English](../en_US/topic-rewrite.md)  | 简体中文

# 主题重写

一些早期老旧的物联网设备不支持重新配置或升级，修改设备MQTT订阅主题会非常困难。主题重写功能可以通过给 *RMQTT* 设置一套规则，
它可以在订阅/退订/发布时将原有主题重写为新的目标主题。


* 由于发布/订阅授权检查会在主题重写之后执行，所以须要确保重写之后的主题能够通过 ACL 检查即可。


* 主题重写在作用于客户端的订阅/取消订阅共享订阅主题时，仅对实际主题生效。即只对共享订阅主题去除前缀 $share/<group-name>/ 后的部分生效。 
例如：在客户端订阅/取消订阅共享订阅主题过滤器 $share/group/x/y/z 时，仅尝试匹配并重写x/y/z，忽略 $share/group/。

#### 配置主题重写规则

*RMQTT* 的主题重写规则需要用户自行配置，用户可以自行添加多条主题重写规则，规则的数量没有限制，但。。。

每条主题重写规则的格式如下：
```bash
rules = [
    { action = "all", source_topic_filter = "x/+/#", dest_topic = "xx/$1/$2", regex = "^x/(.+)/(.+)$" }
]
```

每个重写规则由过滤器、目标表达式和正则表达式组成。

重写规则分为 publish 、subscribe 和 all 规则，publish 规则匹配 PUBLISH 报文携带的主题，subscribe 规则匹配 SUBSCRIBE、UNSUBSCRIBE 
报文携带的主题。all 规则对 PUBLISH、SUBSCRIBE 和 UNSUBSCRIBE 报文携带的主题都生效。

在启用主题重写的前提下，当收到 MQTT 数据包（如带有主题的PUBLISH消息）时，RMQTT 将使用数据包中的主题来匹配配置文件中规则的主题过滤器部分。
匹配成功之后，正则表达式就会被用来提取主题中的信息，然后用目标表达式替换旧的主题，生成一个新的主题。

目标表达式可以使用 $N 格式的变量来匹配从正则表达式中提取的元素。$N 的值是指从正则表达式中提取的第 N 个元素，例如，$1 是正则表达式提取的第一个元素。

同时，表达式中也可以使用 ${clientid} 代表 客户端Id, 使用 ${username} 代表 客户端用户名。

注意：RMQTT 会使用规则配置的主题过滤器来构建搜索主题树，当一个主题可以同时匹配多个主题重写规则的主题过滤器时，RMQTT 仅使用最后匹配成功的规则来重写该主题。

如果规则中的正则表达式与 MQTT 数据包的主题不匹配，则重写失败，其他规则将不会被用来重写。因此，需要仔细设计 MQTT 数据包主题和主题重写规则。



#### 插件：

```bash
rmqtt-topic-rewrite
```

#### 插件配置文件：

```bash
plugins/rmqtt-topic-rewrite.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-topic-rewrite
##--------------------------------------------------------------------

# action - Options are publish, subscribe, or all
# source_topic_filter - Topic filter for subscribing, unsubscribing, or publishing messages
# dest_topic - Destination topic
# regex - Regular expression

# Expressions can use ${clientid} to represent the client ID and ${username} to represent the client username.

rules = [
    { action = "all", source_topic_filter = "x/+/#", dest_topic = "xx/$1/$2", regex = "^x/(.+)/(.+)$" },
    { action = "all", source_topic_filter = "x/y/1", dest_topic = "xx/y/1" },
    { action = "all", source_topic_filter = "x/y/#", dest_topic = "xx/y/$1", regex = "^x/y/(.+)$" },
    { action = "all", source_topic_filter = "iot/cid/#", dest_topic = "iot/${clientid}/$1", regex = "^iot/cid/(.+)$" },
    { action = "all", source_topic_filter = "iot/uname/#", dest_topic = "iot/${username}/$1", regex = "^iot/uname/(.+)$" },
    { action = "all", source_topic_filter = "a/+/+/+", dest_topic = "aa/$1/$2/$3", regex = "^a/(.+)/(.+)/(.+)$" }
]
```

配置了六条主题重写规则。 x/+/#, x/y/1, x/y/#, iot/cid/#, iot/uname/# 和 a/+/+/+ 。

* z/def 不符合任何主题过滤器，所以它不执行主题重写，只是订阅或发布 z/def 主题。
* x/a/z 匹配 x/+/# 主题过滤器，RMQTT 执行第一条规则，并通过正则表达式匹配元素 [a、z] ，将匹配的第一个和第二个元素带入 xx/$1/$2 ，并实际订阅或发布主题为： xx/a/z
* x/y/1 同时匹配 x/+/#、x/y/# 和 x/y/1 三个主题过滤器，最后匹配成功的是：x/y/1，因为没有配置正则表达式，所以实际订阅或发布主题为： xx/y/1
* x/y/2 同时匹配 x/+/# 和 x/y/# 两个主题过滤器，最后匹配成功的是：x/y/#，通过正则替换，它实际上订阅或发布主题为： xx/y/2
* iot/cid/1 匹配 iot/cid/# 主题过滤器，通过正则替换和clientid替换，实际上订阅或发布主题为：iot/cid001/1
* iot/uname/1 匹配 iot/uname/# 主题过滤器，通过正则替换和username替换，实际上订阅或发布主题为：iot/uname001/1
* a/1/2/3 匹配 a/+/+/+ 主题过滤器，通过正则替换，实际上订阅或发布主题为：aa/1/2/3
* a/1/2/3/4 没能匹配上任何主题重写规则

默认情况下并没有启动此插件，如果要开启会话存储插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-topic-rewrite”项，如：
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
    "rmqtt-topic-rewrite",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

