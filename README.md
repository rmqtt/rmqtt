# RMQTT Broker

[![GitHub Release](https://img.shields.io/github/release/rmqtt/rmqtt?color=brightgreen)](https://github.com/rmqtt/rmqtt/releases)
<a href="https://blog.rust-lang.org/2025/03/18/Rust-1.85.1/"><img alt="Rust Version" src="https://img.shields.io/badge/rust-1.85.0%2B-blue" /></a>
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/rmqtt/rmqtt)
[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
[![docs.rs page](https://docs.rs/rmqtt/badge.svg)](https://docs.rs/rmqtt/latest/rmqtt/)


English | [简体中文](./README-CN.md)

*RMQTT* broker is a fully open source, highly scalable, highly available distributed MQTT messaging broker for IoT, M2M
and mobile applications that can handle millions of concurrent clients on a single service node.

## Features

- 100% Rust safe code;
- Based on [tokio](https://crates.io/crates/tokio);
- MQTT v3.1, v3.1.1 and v5.0 protocols support;
    - QoS0, QoS1, QoS2 message support;
    - Offline message support;
    - [Retained message support](./docs/en_US/retainer.md);
    - Last Will message support;
- [Distributed cluster](./docs/en_US/cluster-raft.md);
- [Built-in AUTH/ACL](./docs/en_US/acl.md);
- [HTTP AUTH/ACL](./docs/en_US/auth-http.md);
- [JWT AUTH/ACL](./docs/en_US/auth-jwt.md);
- [WebHook](./docs/en_US/web-hook.md);
- [HTTP APIs](./docs/en_US/http-api.md);
- [$SYS System Topics](./docs/en_US/sys-topic.md);
- [Store session information](./docs/en_US/store-session.md);
- [Store unexpired messages](./docs/en_US/store-message.md);
- [MQTT Bridging - Ingress Mode](./docs/en_US/bridge-ingress-mqtt.md)
- [MQTT Bridging - Egress Mode](./docs/en_US/bridge-egress-mqtt.md)
- [Apache Kafka Bridging - Ingress Mode](./docs/en_US/bridge-ingress-kafka.md)
- [Apache Kafka Bridging - Egress Mode](./docs/en_US/bridge-egress-kafka.md)
- [Apache Pulsar Bridging - Ingress Mode](./docs/en_US/bridge-ingress-pulsar.md)
- [Apache Pulsar Bridging - Egress Mode](./docs/en_US/bridge-egress-pulsar.md)
- [NATS Bridging - Ingress Mode](./docs/en_US/bridge-ingress-nats.md)
- [NATS Bridging - Egress Mode](./docs/en_US/bridge-egress-nats.md)
- [Reductstore Bridging - Egress Mode](./docs/en_US/bridge-egress-reductstore.md)
- [Topic Rewrite](./docs/en_US/topic-rewrite.md)
- [Auto Subscription](./docs/en_US/auto-subscription.md)
- [P2P Messaging](./docs/en_US/p2p-messaging.md)
- Shared subscription($share/{Group}/{TopicFilter});
- Exclusive subscription($exclusive/{TopicFilter});
- Limit subscription($limit/{LimitQuantity}/{TopicFilter});
- Delayed publish($delayed/{DelayInterval}/{TopicName});
- Hooks;
- TLS support;
- WebSocket support;
- WebSocket-TLS support;
- Built-in extensible components;
- Extensible plug-in support;
- Metrics & Stats;
- Rate limit;
- Inflight and Queue;
- Message resending;

- For full list of new features, please read [RMQTT Release Notes](https://github.com/rmqtt/rmqtt/releases).

## Installation

The *RMQTT* broker is cross-platform, which supports Linux, Unix, macOS and Windows. It means *RMQTT* can be deployed on
x86_64 architecture servers and ARM devices like Raspberry Pi.

#### Run RMQTT using Docker

* Single node

```bash
mkdir -p /app/log/rmqtt
docker run -d --name rmqtt -p 1883:1883 -p 8883:8883 -p 11883:11883 -p 6060:6060 -v /app/log/rmqtt:/var/log/rmqtt  rmqtt/rmqtt:latest
```

* Multi node

```bash
  docker run -d --name rmqtt1 -p 1884:1883 -p 8884:8883 -p 11884:11883 -p 6064:6060 -v /app/log/rmqtt/1:/var/log/rmqtt  rmqtt/rmqtt:latest --id 1 --plugins-default-startups "rmqtt-cluster-raft" --node-grpc-addrs "1@172.17.0.3:5363" "2@172.17.0.4:5363" "3@172.17.0.5:5363" --raft-peer-addrs "1@172.17.0.3:6003" "2@172.17.0.4:6003" "3@172.17.0.5:6003"   

  docker run -d --name rmqtt2 -p 1885:1883 -p 8885:8883 -p 11885:11883 -p 6065:6060 -v /app/log/rmqtt/2:/var/log/rmqtt  rmqtt/rmqtt:latest --id 2 --plugins-default-startups "rmqtt-cluster-raft" --node-grpc-addrs "1@172.17.0.3:5363" "2@172.17.0.4:5363" "3@172.17.0.5:5363" --raft-peer-addrs "1@172.17.0.3:6003" "2@172.17.0.4:6003" "3@172.17.0.5:6003"   

  docker run -d --name rmqtt3 -p 1886:1883 -p 8886:8883 -p 11886:11883 -p 6066:6060 -v /app/log/rmqtt/3:/var/log/rmqtt  rmqtt/rmqtt:latest --id 3 --plugins-default-startups "rmqtt-cluster-raft" --node-grpc-addrs "1@172.17.0.3:5363" "2@172.17.0.4:5363" "3@172.17.0.5:5363" --raft-peer-addrs "1@172.17.0.3:6003" "2@172.17.0.4:6003" "3@172.17.0.5:6003"
```

Node IDs: 1, 2, 3; Node IP Addrs: 172.17.0.3, 172.17.0.4, 172.17.0.5

#### Create a static cluster by docker-compose

1. [Download docker-compose configuration template](https://github.com/rmqtt/templates/blob/main/docker-compose-template/docker-compose-template-v0.7.zip)

2. Start docker-compose cluster

```bash
docker-compose up -d
```

3. View cluster

```bash
curl "http://127.0.0.1:6066/api/v1/health/check"
```

#### Installing via ZIP Binary Package (Linux、MacOS、Windows)

Get the binary package of the corresponding OS from [RMQTT Download](https://github.com/rmqtt/rmqtt/releases) page.

- [Single Node Install](./docs/en_US/install.md)

- [Multi Node Install](./docs/en_US/install.md)


## Library Mode Integration

In addition to running as a standalone MQTT Broker/Server, rmqtt also provides a **Library Mode**, which allows you to embed rmqtt directly into your Rust applications or services. Simply add the following dependency to your `Cargo.toml`, and you can use rmqtt's APIs just like a regular Rust library:

```toml
[dependencies]
rmqtt = "0.16"
```

For more details about using rmqtt in library mode, please refer to the [RMQTT Library Documentation](./rmqtt/README.md).

## Experience

[//]: # (- MQTT Broker: 47.103.110.134:1883)

[//]: # (- Account:)

[//]: # (- HTTP APIs: http://47.103.110.134:6080/api/v1/)

## Test

### Functional Testing

#### paho.mqtt.testing(MQTT V3.1.1) [client_test.py](https://github.com/eclipse/paho.mqtt.testing/blob/master/interoperability/client_test.py)

* client_test.py Test.test_retained_messages          [OK]
* client_test.py Test.test_zero_length_clientid       [OK]
* client_test.py Test.will_message_test               [OK]
* client_test.py Test.test_zero_length_clientid       [OK]
* client_test.py Test.test_offline_message_queueing   [OK]
* client_test.py Test.test_overlapping_subscriptions  [OK]
* client_test.py Test.test_keepalive                  [OK]
* client_test.py Test.test_redelivery_on_reconnect    [OK]
* client_test.py Test.test_dollar_topics              [OK]
* client_test.py Test.test_unsubscribe                [OK]
* client_test.py Test.test_subscribe_failure          [OK]  
  * You need to modify the `rmqtt-acl.toml` configuration and add the following line at the first line: ["deny", "all", "subscribe", ["test/nosubscribe"]]

#### paho.mqtt.testing(MQTT V5.0) [client_test5.py](https://github.com/eclipse/paho.mqtt.testing/blob/master/interoperability/client_test5.py)

* client_test5.py Test.test_retained_message            [OK]
* client_test5.py Test.test_will_message                [OK]
* client_test5.py Test.test_offline_message_queueing    [OK]
* client_test5.py Test.test_dollar_topics               [OK]
* client_test5.py Test.test_unsubscribe                 [OK]
* client_test5.py Test.test_session_expiry              [OK]
* client_test5.py Test.test_shared_subscriptions        [OK]
* client_test5.py Test.test_basic                       [OK]
* client_test5.py Test.test_overlapping_subscriptions   [OK]
* client_test5.py Test.test_redelivery_on_reconnect     [OK]
* client_test5.py Test.test_payload_format              [OK]
* client_test5.py Test.test_publication_expiry          [OK]
* client_test5.py Test.test_subscribe_options           [OK]
* client_test5.py Test.test_assigned_clientid           [OK]
* client_test5.py Test.test_subscribe_identifiers       [OK]
* client_test5.py Test.test_request_response            [OK]
* client_test5.py Test.test_server_topic_alias          [OK]
* client_test5.py Test.test_client_topic_alias          [OK]
* client_test5.py Test.test_maximum_packet_size         [OK]
* client_test5.py Test.test_keepalive                   [OK]
* client_test5.py Test.test_zero_length_clientid        [OK]
* client_test5.py Test.test_user_properties             [OK]
* client_test5.py Test.test_flow_control2               [OK]
* client_test5.py Test.test_flow_control1               [OK]
* client_test5.py Test.test_will_delay                  [OK]
* client_test5.py Test.test_server_keep_alive           [OK]
  * You need to modify the `rmqtt.toml` configuration and change `max_keepalive` to 60.
* client_test5.py Test.test_subscribe_failure           [OK]
  * You need to modify the `rmqtt-acl.toml` configuration and add the following line at the first line: ["deny", "all", "subscribe", ["test/nosubscribe"]]


### Benchmark Testing

#### environment
| Item        | Content                                   |                                                              |
|-------------|-------------------------------------------|--------------------------------------------------------------|
| System      | x86_64 GNU/Linux                          | Rocky Linux 9.2 (Blue Onyx)                                  |
| CPU         | Intel(R) Xeon(R) CPU E5-2696 v3 @ 2.30GHz | 72(CPU(s)) = 18(Core(s)) * 2(Thread(s) per core) * 2(Socket(s)) |
| Memory      | DDR3/2333                                 | 128G                                                         |
| Disk        |                                           | 2T                                                           |
| Container   | podman                                    | v4.4.1                                                       |
| MQTT Bench  | docker.io/rmqtt/rmqtt-bench:latest        | v0.1.3                                                       |
| MQTT Broker | docker.io/rmqtt/rmqtt:latest              | v0.3.0                                                       |
| Other       | MQTT Bench and MQTT Broker coexistence    |                                                              |

#### Connection Concurrency Performance
| Item                  | Single Node       | Raft Cluster Mode |
|-----------------------|-------------------|--------------------|
| Total Concurrent Clients | 1,000,000       | 1,000,000          |
| Connection Handshake Rate | (5500-7000)/second | (5000-7000)/second |

#### Message Throughput Performance
| Item                    | Single Node         | Raft Cluster Mode |
|-------------------------|---------------------|--------------------|
| Subscription Client Count | 1,000,000          | 1,000,000          |
| Publishing Client Count   | 40                | 40                 |
| Message Throughput Rate  | 150,000/second      | 156,000/second     |

[For detailed benchmark test results and information, see documentation.](./docs/en_US/benchmark-testing.md)

## ⭐ Star History
[![Star History Chart](https://api.star-history.com/svg?repos=rmqtt/rmqtt&type=Date)](https://star-history.com/#rmqtt/rmqtt&Date)

