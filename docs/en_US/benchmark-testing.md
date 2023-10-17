English | [简体中文](../zh_CN/benchmark-testing.md)


# Hardware and software environment
| Item        | Content                                   |                                                                 |
|-------------|-------------------------------------------|-----------------------------------------------------------------|
| System      | x86_64 GNU/Linux                          | Rocky Linux 9.2 (Blue Onyx)                                     |
| CPU         | Intel(R) Xeon(R) CPU E5-2696 v3 @ 2.30GHz | 72(CPU(s)) = 18(Core(s)) * 2(Thread(s) per core) * 2(Socket(s)) |
| Memory      | DDR3/2333                                 | 128G                                                            |
| Disk        |                                           | 2T                                                              |
| Container   | podman                                    | v4.4.1                                                          |
| MQTT Bench  | docker.io/rmqtt/rmqtt-bench:latest        | v0.1.3                                                          |
| MQTT Broker | docker.io/rmqtt/rmqtt:latest              | v0.2.18                                                         |
| Other       | MQTT Bench and MQTT Broker coexistence    |                                                                 |


# Standalone Mode

Broker Main Configuration (rmqtt.toml):
```
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "/app/rmqtt/etc/plugins"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-auth-http",
    #"rmqtt-web-hook",
    #"rmqtt-retainer",
    "rmqtt-http-api"
]

##--------------------------------------------------------------------
## Listeners
##--------------------------------------------------------------------

##--------------------------------------------------------------------
## MQTT/TCP - External TCP Listener for MQTT Protocol
listener.tcp.external.addr = "0.0.0.0:1883"
listener.tcp.external.workers = 68
listener.tcp.external.max_connections = 1024000
listener.tcp.external.max_handshaking_limit = 1500   #Single instance set to 1500, cluster version set to 500.
listener.tcp.external.handshake_timeout = "30s"
listener.tcp.external.max_packet_size = "1m"
listener.tcp.external.backlog = 1024
listener.tcp.external.allow_anonymous = true
listener.tcp.external.allow_zero_keepalive = true
listener.tcp.external.min_keepalive = 0
listener.tcp.external.max_keepalive = 65535
listener.tcp.external.keepalive_backoff = 0.75
listener.tcp.external.max_inflight = 16
listener.tcp.external.max_mqueue_len = 1000
listener.tcp.external.mqueue_rate_limit = "1000,1s"
listener.tcp.external.max_clientid_len = 65535
listener.tcp.external.max_qos_allowed = 2
listener.tcp.external.max_topic_levels = 0
listener.tcp.external.retain_available = true
listener.tcp.external.session_expiry_interval = "2h"
listener.tcp.external.message_retry_interval = "20s"
listener.tcp.external.message_expiry_interval = "5m"
listener.tcp.external.max_subscriptions = 0
listener.tcp.external.shared_subscription = true
listener.tcp.external.max_topic_aliases = 32

```

Create a bridge network:
```
podman network create compose_rmqtt_bridge1
```

Podman command for a single-instance Broker:
```
podman run  -d --name rmqtt1 --privileged --ulimit nofile=1024000 --network="compose_rmqtt_bridge1" --network-alias="node1.rmqtt.io" -p 1884:1883 -p 8884:8883 -p 21884:11883 -p 7064:6060 -v /home/dev/app/compose/log/1:/var/log/rmqtt -v /home/dev/app/compose/etc:/app/rmqtt/etc  rmqtt/rmqtt:latest -f ./etc/rmqtt.toml --id 1
```

Generate a 'podman-compose' configuration script for a single-instance subscription client:

build-bench-sub-single.sh
```
#!/bin/bash

n=$1
start=$2

echo "version: '3'
services :"

for ((i=$start; i<$start+$n; i++))
do
  echo "
  mqtt-bench-a$i:
    image: rmqtt/rmqtt-bench:latest
    restart: on-failure:3
    command: v3 -C -c 25000 -E \"sub-a$i-{no}\" -S -t iot/a$i-{no} --addrs \"node1.rmqtt.io:1883\" --ifaddrs \"a$i.rmqtt-bench.io\"
    networks:
      rmqtt_bridge1: 
        aliases:
        - a$i.rmqtt-bench.io"

done

echo "networks:
  rmqtt_bridge1:
    driver: bridge"
```


Create docker-compose.yaml
```
sh build-bench-sub-single.sh 10   1 > docker-compose-bench-subs-single-10-1.yaml
sh build-bench-sub-single.sh 10   11 > docker-compose-bench-subs-single-10-11.yaml
sh build-bench-sub-single.sh 10   21 > docker-compose-bench-subs-single-10-21.yaml
sh build-bench-sub-single.sh 10   31 > docker-compose-bench-subs-single-10-31.yaml
```
One container creates 25,000 connections, and there are 10 containers in one file, for a total of 1 million connections.


Generate a 'podman-compose' configuration script for a single-instance publishing client:

build-bench-pub-single.sh
```
#!/bin/bash

n=$1
start=$2

echo "version: '3'
services :"

for ((i=$start; i<$start+$n; i++))
do
  echo "
  mqtt-bench-pub-a$i:
    image: rmqtt/rmqtt-bench:latest
    restart: on-failure:3
    command: v3 -C -c 1 -E \"pub-a$i-{no}\" -P -t iot/a$i-{no} -R 0 24999  -I 0 --addrs \"node1.rmqtt.io:1883\" --ifaddrs \"a$i.rmqtt-bench-pub.io\"
    networks:
      rmqtt_bridge1: 
        aliases:
        - a$i.rmqtt-bench-pub.io"

done

echo "networks:
  rmqtt_bridge1:
    driver: bridge"
```

Create docker-compose.yaml
```
sh build-bench-pub-single.sh 40   1 > docker-compose-bench-pubs-single-40-1.yaml
```
One container creates 1 connection and randomly sends messages in a loop to 25,000 subscribing clients. In total, 40 connections are established.


## Connection concurrency performance

After successful connection, subscriptions are initiated simultaneously. Connections are established in four batches, with each batch consisting of 250,000 clients initiating connection requests simultaneously.

| Item                               | Value              | Description |
|------------------------------------|--------------------|----------------|
| Total number of concurrent clients | 1,000,000          | 25000 * 10 * 4 |
| Connection Handshake Rate          | (5500-7000)/second | Average Rate         |
| Broker Process Memory              | 33.2G              |                      |
| Broker Process CPU                 | (1-200)%           |                      |
| Bench-subs Process Memory          | 400M * 40          |                      |
| Bench-subs Process CPU             | (0-20)% * 40       |                      |

To check the status:
```
curl "http://127.0.0.1:7064/api/v1/stats" | jq
[
  {
    "node": {
      "id": 1,
      "name": "1@127.0.0.1",
      "status": "Running"
    },
    "stats": {
      "connections.count": 1000000,
      "connections.max": 1000000,
      "handshakings.count": 0,
      "handshakings.max": 62,
      "handshakings_active.count": 0,
      "handshakings_rate.count": 7282.29,
      "handshakings_rate.max": 16600.09,
      "inflights.count": 0,
      "inflights.max": 0,
      "message_queues.count": 0,
      "message_queues.max": 0,
      "retained.count": 0,
      "retained.max": 0,
      "routes.count": 1000000,
      "routes.max": 1000000,
      "sessions.count": 1000000,
      "sessions.max": 1000000,
      "subscriptions.count": 1000000,
      "subscriptions.max": 1000000,
      "subscriptions_shared.count": 0,
      "subscriptions_shared.max": 0,
      "topics.count": 1000000,
      "topics.max": 1000000
    }
  }
]

```

## Message Throughput Performance

| Item                               | Value          | Description |
|-----------------|----------------|----------------|
| Subscription Client Count | 1,000,000      |                  |
| Publishing Client Count   | 40             |                  |
| Message Throughput Rate   | 150,000/second |                  |
| Broker Process Memory    | 44.5G          |                  |
| Broker Process CPU       | (1800-1900)%   |                  |
| Bench-subs Process Memory | 430M * 40      |                  |
| Bench-subs Process CPU   | (20-30)% * 40  |                  |
| Bench-pubs Process Memory | 16M * 40       |                  |
| Bench-pubs Process CPU   | (20-30)% * 40  |                  |

To view the status::
```
curl "http://127.0.0.1:7064/api/v1/stats" | jq
[
  {
    "node": {
      "id": 1,
      "name": "1@127.0.0.1",
      "status": "Running"
    },
    "stats": {
      "connections.count": 1000040,
      "connections.max": 1000040,
      "handshakings.count": 0,
      "handshakings.max": 67,
      "handshakings_active.count": 0,
      "handshakings_rate.count": 399.69,
      "handshakings_rate.max": 25261.64,
      "inflights.count": 5480,
      "inflights.max": 19407,
      "message_queues.count": 1,
      "message_queues.max": 33,
      "retained.count": 0,
      "retained.max": 0,
      "routes.count": 1000000,
      "routes.max": 1000000,
      "sessions.count": 1000040,
      "sessions.max": 1000040,
      "subscriptions.count": 1000000,
      "subscriptions.max": 1000000,
      "subscriptions_shared.count": 0,
      "subscriptions_shared.max": 0,
      "topics.count": 1000000,
      "topics.max": 1000000
    }
  }
]
```

Message Statistics:
```
date & curl "http://127.0.0.1:7064/api/v1/metrics" | jq | grep messages
Wed Sep  6 21:07:15 CST 2023

      "messages.acked": 1321545894,
      "messages.acked.admin": 0,
      "messages.acked.custom": 1321545895,
      "messages.acked.lastwill": 0,
      "messages.acked.retain": 0,
      "messages.acked.system": 0,
      "messages.delivered": 1321545978,
      "messages.delivered.admin": 0,
      "messages.delivered.custom": 1321545979,
      "messages.delivered.lastwill": 0,
      "messages.delivered.retain": 0,
      "messages.delivered.system": 0,
      "messages.dropped": 0,
      "messages.nonsubscribed": 0,
      "messages.nonsubscribed.admin": 0,
      "messages.nonsubscribed.custom": 0,
      "messages.nonsubscribed.lastwill": 0,
      "messages.nonsubscribed.system": 0,
      "messages.publish": 1321546048,
      "messages.publish.admin": 0,
      "messages.publish.custom": 1321546048,
      "messages.publish.lastwill": 0,
      "messages.publish.system": 0,
```
```
 date & curl "http://127.0.0.1:7064/api/v1/metrics" | jq | grep messages
Wed Sep  6 21:07:54 CST 2023

      "messages.acked": 1327420995,
      "messages.acked.admin": 0,
      "messages.acked.custom": 1327420995,
      "messages.acked.lastwill": 0,
      "messages.acked.retain": 0,
      "messages.acked.system": 0,
      "messages.delivered": 1327421038,
      "messages.delivered.admin": 0,
      "messages.delivered.custom": 1327421039,
      "messages.delivered.lastwill": 0,
      "messages.delivered.retain": 0,
      "messages.delivered.system": 0,
      "messages.dropped": 0,
      "messages.nonsubscribed": 0,
      "messages.nonsubscribed.admin": 0,
      "messages.nonsubscribed.custom": 0,
      "messages.nonsubscribed.lastwill": 0,
      "messages.nonsubscribed.system": 0,
      "messages.publish": 1327421058,
      "messages.publish.admin": 0,
      "messages.publish.custom": 1327421060,
      "messages.publish.lastwill": 0,
      "messages.publish.system": 0,
```

# Raft Cluster Mode

Broker Main Configuration (rmqtt.toml):
```
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "/app/rmqtt/etc/plugins"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-cluster-broadcast",
    "rmqtt-cluster-raft",
    #"rmqtt-auth-http",
    #"rmqtt-web-hook",
    #"rmqtt-retainer",
    "rmqtt-http-api"
]

##--------------------------------------------------------------------
## Listeners
##--------------------------------------------------------------------

##--------------------------------------------------------------------
## MQTT/TCP - External TCP Listener for MQTT Protocol
listener.tcp.external.addr = "0.0.0.0:1883"
listener.tcp.external.workers = 24
listener.tcp.external.max_connections = 1024000
listener.tcp.external.max_handshaking_limit = 500
listener.tcp.external.handshake_timeout = "30s"
listener.tcp.external.max_packet_size = "1m"
listener.tcp.external.backlog = 1024
listener.tcp.external.allow_anonymous = true
listener.tcp.external.allow_zero_keepalive = true
listener.tcp.external.min_keepalive = 0
listener.tcp.external.max_keepalive = 65535
listener.tcp.external.keepalive_backoff = 0.75
listener.tcp.external.max_inflight = 16
listener.tcp.external.max_mqueue_len = 1000
listener.tcp.external.mqueue_rate_limit = "1000,1s"
listener.tcp.external.max_clientid_len = 65535
listener.tcp.external.max_qos_allowed = 2
listener.tcp.external.max_topic_levels = 0
listener.tcp.external.retain_available = true
listener.tcp.external.session_expiry_interval = "2h"
listener.tcp.external.message_retry_interval = "20s"
listener.tcp.external.message_expiry_interval = "5m"
listener.tcp.external.max_subscriptions = 0
listener.tcp.external.shared_subscription = true
listener.tcp.external.max_topic_aliases = 32

```
The main difference from the single-node mode is the addition of the 'rmqtt-cluster-raft' plugin. Modify the 'workers' and 'max_handshaking_limit' configuration options.

Raft Cluster Plugin Configuration(rmqtt-cluster-raft.toml):
```
#Node GRPC service address list
node_grpc_addrs = ["1@node14.rmqtt.io:5363","2@node15.rmqtt.io:5363","3@node16.rmqtt.io:5363"]
#Raft peer address list
raft_peer_addrs = ["1@node14.rmqtt.io:16003","2@node15.rmqtt.io:16003","3@node16.rmqtt.io:16003"]
```
The address needs to match the address used in the Podman command.

Create a bridge network:
```
podman network create compose_rmqtt_bridge11
```

Podman command for the Broker:
```
podman run -d --name rmqtt1 --privileged --ulimit nofile=1024000 --network="compose_rmqtt_bridge11" --network-alias="node14.rmqtt.io" -p 1884:1883 -p 8884:8883 -p 21884:11883 -p 7064:6060 -v /home/dev/app/compose/log/1:/var/log/rmqtt -v /home/dev/app/compose/etc:/app/rmqtt/etc  rmqtt/rmqtt:latest -f ./etc/rmqtt.toml --id 1
podman run -d --name rmqtt2 --privileged --ulimit nofile=1024000 --network="compose_rmqtt_bridge11" --network-alias="node15.rmqtt.io"  -p 1885:1883 -p 8885:8883 -p 11885:11883 -p 7065:6060 -v /home/try/app/compose/log/2:/var/log/rmqtt -v /home/try/app/compose/etc:/app/rmqtt/etc  rmqtt/rmqtt:latest -f ./etc/rmqtt.toml --id 2
podman run -d --name rmqtt3 --privileged --ulimit nofile=1024000 --network="compose_rmqtt_bridge11" --network-alias="node16.rmqtt.io"  -p 1886:1883 -p 8886:8883 -p 11886:11883 -p 7066:6060 -v /home/try/app/compose/log/3:/var/log/rmqtt -v /home/try/app/compose/etc:/app/rmqtt/etc  rmqtt/rmqtt:latest -f ./etc/rmqtt.toml --id 3
```


Generate a 'podman-compose' configuration script for the subscription client:

build-bench-sub.sh
```
#!/bin/bash

n=$1
start=$2

echo "version: '3'
services :"

for ((i=$start; i<$start+$n; i++))
do
  echo "
  mqtt-bench-a$i:
    image: rmqtt/rmqtt-bench:latest
    restart: on-failure:3
    command: v3 -C -c 25000 -E \"sub-a$i-{no}\" -S -t iot/a$i-{no} --addrs \"node14.rmqtt.io:1883\" \"node15.rmqtt.io:1883\" \"node16.rmqtt.io:1883\" --ifaddrs \"a$i.rmqtt-bench.io\"
    networks:
      rmqtt_bridge11: 
        aliases:
        - a$i.rmqtt-bench.io"

done

echo "networks:
  rmqtt_bridge11:
    driver: bridge"
```

Create docker-compose.yaml
```
sh build-bench-sub.sh 10 1 > docker-compose-bench-subs-10-1.yaml
sh build-bench-sub.sh 10 11 > docker-compose-bench-subs-10-11.yaml
sh build-bench-sub.sh 10 21 > docker-compose-bench-subs-10-21.yaml
sh build-bench-sub.sh 10 31 > docker-compose-bench-subs-10-31.yaml
```
One container creates 25,000 connections, and there are 10 containers in one file, for a total of 1 million connections.


Generate a 'podman-compose' configuration script for the publishing client:

build-bench-pub.sh
```
#!/bin/bash

n=$1
start=$2

echo "version: '3'
services :"

for ((i=$start; i<$start+$n; i++))
do
  echo "
  mqtt-bench-pub-a$i:
    image: rmqtt/rmqtt-bench:latest
    restart: on-failure:3
    command: v3 -C -c 1 -E \"pub-a$i-{no}\" -P -t iot/a$i-{no} -R 0 24999  -I 0 --addrs \"node14.rmqtt.io:1883\" \"node15.rmqtt.io:1883\" \"node16.rmqtt.io:1883\" --ifaddrs \"a$i.rmqtt-bench-pub.io\"
    networks:
      rmqtt_bridge11: 
        aliases:
        - a$i.rmqtt-bench-pub.io"

done


echo "networks:
  rmqtt_bridge11:
    driver: bridge"
```

Create docker-compose.yaml
```
sh build-bench-pub.sh 40   1 > docker-compose-bench-pubs-40-1.yaml 
```
One container creates 1 connection and sends messages in a loop to 25,000 subscribing clients at random, with a total of 40 connections.


## Connection Concurrency Performance
Subscriptions are initiated simultaneously upon successful connection. Connections are established in four batches, with each batch consisting of 250,000 clients initiating connection requests simultaneously.

| Item               | Value            | Description       |
| ------------------ |------------------| ----------------- |
| Total Concurrent Clients | 1,000,000        | 25000 * 10 * 4    |
| Connection Handshake Rate | (5000-7000)/second | Average Rate   |
| Broker Process Memory | 12.9G * 3        | Three nodes       |
| Broker Process CPU    | (1-90)% * 3      | Three nodes       |
| Bench-subs Process Memory | 400M * 40        |                   |
| Bench-subs Process CPU  | (0-20)% * 40     |                   |


To check the status:
```
curl "http://127.0.0.1:7064/api/v1/stats/sum" | jq
{
  "nodes": {
    "1": {
      "name": "1@node14.rmqtt.io:5363",
      "status": "Running"
    },
    "2": {
      "name": "2@node15.rmqtt.io:5363",
      "status": "Running"
    },
    "3": {
      "name": "3@node16.rmqtt.io:5363",
      "status": "Running"
    }
  },
  "stats": {
    "connections.count": 1000000,
    "connections.max": 1000001,
    "handshakings.count": 0,
    "handshakings.max": 77220,
    "handshakings_active.count": 0,
    "handshakings_rate.count": 2063.15,
    "handshakings_rate.max": 52652.54,
    "inflights.count": 0,
    "inflights.max": 0,
    "message_queues.count": 0,
    "message_queues.max": 0,
    "retained.count": 0,
    "retained.max": 0,
    "routes.count": 1000000,
    "routes.max": 1000000,
    "sessions.count": 1000000,
    "sessions.max": 1000001,
    "subscriptions.count": 1000000,
    "subscriptions.max": 1000000,
    "subscriptions_shared.count": 0,
    "subscriptions_shared.max": 0,
    "topics.count": 1000000,
    "topics.max": 1000000
  }
}
```

## Message Throughput Performance

| Item               | Value          | Description                                |
| ------------------ |----------------| ------------------------------------------ |
| Subscription Client Count | 1,000,000      |                                          |
| Publishing Client Count  | 40             |                                          |
| Message Throughput Rate  | 156,000/second |                                       |
| Broker Process Memory   | 16.6G * 3      | Three nodes                              |
| Broker Process CPU      | (3000-3300)%   | (1000-1100)% + (1200-1300)% + (800-900)% |
| Bench-subs Process Memory | 430M * 40      |                                          |
| Bench-subs Process CPU | (20-30)% * 40  |                                       |
| Bench-pubs Process Memory | 16M * 40       |                                          |
| Bench-pubs Process CPU  | (20-30)% * 40  |                                       |

To check the status:
```
curl "http://127.0.0.1:7064/api/v1/stats/sum" | jq
{
  "nodes": {
    "1": {
      "name": "1@node14.rmqtt.io:5363",
      "status": "Running"
    },
    "2": {
      "name": "2@node15.rmqtt.io:5363",
      "status": "Running"
    },
    "3": {
      "name": "3@node16.rmqtt.io:5363",
      "status": "Running"
    }
  },
  "stats": {
    "connections.count": 1000040,
    "connections.max": 1000040,
    "handshakings.count": 1,
    "handshakings.max": 77220,
    "handshakings_active.count": 0,
    "handshakings_rate.count": 2063.15,
    "handshakings_rate.max": 52652.54,
    "inflights.count": 74,
    "inflights.max": 16913,
    "message_queues.count": 1,
    "message_queues.max": 26,
    "retained.count": 0,
    "retained.max": 0,
    "routes.count": 1000000,
    "routes.max": 1000000,
    "sessions.count": 1000040,
    "sessions.max": 1000040,
    "subscriptions.count": 1000000,
    "subscriptions.max": 1000000,
    "subscriptions_shared.count": 0,
    "subscriptions_shared.max": 0,
    "topics.count": 1000000,
    "topics.max": 1000000
  }
}
```

Message Statistics:
```
date & curl "http://127.0.0.1:7064/api/v1/metrics/sum" | jq | grep messages
Thu Sep  7 11:18:00 CST 2023

  "messages.acked": 39136344,
  "messages.acked.admin": 0,
  "messages.acked.custom": 39136344,
  "messages.acked.lastwill": 0,
  "messages.acked.retain": 0,
  "messages.acked.system": 0,
  "messages.delivered": 39136425,
  "messages.delivered.admin": 0,
  "messages.delivered.custom": 39136425,
  "messages.delivered.lastwill": 0,
  "messages.delivered.retain": 0,
  "messages.delivered.system": 0,
  "messages.dropped": 0,
  "messages.nonsubscribed": 0,
  "messages.nonsubscribed.admin": 0,
  "messages.nonsubscribed.custom": 0,
  "messages.nonsubscribed.lastwill": 0,
  "messages.nonsubscribed.system": 0,
  "messages.publish": 39136867,
  "messages.publish.admin": 0,
  "messages.publish.custom": 39136867,
  "messages.publish.lastwill": 0,
  "messages.publish.system": 0,
```

```
date & curl "http://127.0.0.1:7064/api/v1/metrics/sum" | jq | grep messages
Thu Sep  7 11:18:19 CST 2023

  "messages.acked": 42111887,
  "messages.acked.admin": 0,
  "messages.acked.custom": 42111887,
  "messages.acked.lastwill": 0,
  "messages.acked.retain": 0,
  "messages.acked.system": 0,
  "messages.delivered": 42111997,
  "messages.delivered.admin": 0,
  "messages.delivered.custom": 42111997,
  "messages.delivered.lastwill": 0,
  "messages.delivered.retain": 0,
  "messages.delivered.system": 0,
  "messages.dropped": 0,
  "messages.nonsubscribed": 0,
  "messages.nonsubscribed.admin": 0,
  "messages.nonsubscribed.custom": 0,
  "messages.nonsubscribed.lastwill": 0,
  "messages.nonsubscribed.system": 0,
  "messages.publish": 42112555,
  "messages.publish.admin": 0,
  "messages.publish.custom": 42112555,
  "messages.publish.lastwill": 0,
  "messages.publish.system": 0,
```