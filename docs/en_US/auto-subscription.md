English | [简体中文](../zh_CN/auto-subscription.md)


# Auto Subscription

Auto Subscription allows *RMQTT* to set multiple rules, subscribing the device to specified topics according to the 
rules once it successfully connects, without the need to initiate subscriptions separately.

#### Configuring Auto Subscription Rules

The Auto Subscription rules in *RMQTT* need to be configured by the user, who can add multiple Auto Subscription rules as needed.

The format for each auto subscription rule is as follows:
```bash
subscribes = [
    { topic_filter = "x/+/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 }
]
```

Each override rule consists of a topic filter, QoS, No Local, Retain As Published, and Retain Handling.

Placeholders can be used in the topic filter, where `${clientid}` represents the client ID and `${username}` represents
the client username. For example: `foo/${clientid}/#`.

#### Plugin:

```bash
rmqtt-auto-subscription
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-auto-subscription.toml
```

#### Plugin Configuration Options:

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

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-auto-subscription` entry to the
`plugins.default_startups` configuration in the main configuration file `rmqtt.toml`, as shown below:
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

