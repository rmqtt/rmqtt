# RMQTT Broker

[![GitHub Release](https://img.shields.io/github/release/rmqtt/rmqtt?color=brightgreen)](https://github.com/rmqtt/rmqtt/releases)
<a href="https://blog.rust-lang.org/2023/08/03/Rust-1.71.1.html"><img alt="Rust Version" src="https://img.shields.io/badge/rust-1.71.1%2B-blue" /></a>

[English](./README.md)  | 简体中文

*RMQTT* 是一款完全开源，高度可伸缩，高可用的分布式 MQTT 消息服务器，适用于 IoT、M2M 和移动应用程序，可以在单个服务节点上处理百万级别的并发客户端。

## 功能特色

- 100% Rust安全编码;
- 基于 [tokio](https://crates.io/crates/tokio), [ntex](https://crates.io/crates/ntex)
  , [ntex-mqtt](https://crates.io/crates/ntex-mqtt) 开发;
- 支持MQTT v3.1,v3.1.1 及 v5.0协议;
    - QoS0, QoS1, QoS2 消息支持;
    - 离线消息支持;
    - [Retained 消息支持](./docs/zh_CN/retainer.md);
    - Last Will 消息支持;
- [内置 AUTH/ACL](./docs/zh_CN/acl.md);
- [HTTP AUTH/ACL](./docs/zh_CN/auth-http.md);
- [WebHook](./docs/zh_CN/web-hook.md);
- [HTTP APIs](./docs/zh_CN/http-api.md);
- [$SYS 系统主题](./docs/zh_CN/sys-topic.md);
- [存储会话信息](./docs/zh_CN/store-session.md);
- [存储未过期消息](./docs/zh_CN/store-message.md);
- [MQTT桥接-入口模式](./docs/zh_CN/bridge-ingress-mqtt.md)
- [MQTT桥接-出口模式](./docs/zh_CN/bridge-egress-mqtt.md)
- [Kafka桥接-入口模式](./docs/zh_CN/bridge-ingress-kafka.md)
- [kafka桥接-出口模式](./docs/zh_CN/bridge-egress-kafka.md)
- 分布式集群;
- 钩子(Hooks);
- TLS支持;
- WebSocket支持;
- WebSocket-TLS支持;
- 共享订阅($share/{group}/topic);
- 内置可扩展功能;
- 支持扩展插件;
- 指标监控;
- 速率限制;
- 飞行窗口和消息队列;
- 消息重传;

- 新功能的完整列表，请参阅 [RMQTT Release Notes](https://github.com/rmqtt/rmqtt/releases) 。

## 安装

*RMQTT* 是跨平台的，支持 Linux、Unix、macOS 以及 Windows。这意味着 *RMQTT* 可以部署在 x86_64 架构的服务器上，也可以部署在 Raspberry Pi 这样的 ARM 设备上。

#### 使用 Docker 运行 RMQTT

* 单节点

```bash
mkdir -p /app/log/rmqtt
docker run -d --name rmqtt -p 1883:1883 -p 8883:8883 -p 11883:11883 -p 6060:6060 -v /app/log/rmqtt:/var/log/rmqtt  rmqtt/rmqtt:latest
```

* 多节点

```bash
  docker run -d --name rmqtt1 -p 1884:1883 -p 8884:8883 -p 11884:11883 -p 6064:6060 -v /app/log/rmqtt/1:/var/log/rmqtt  rmqtt/rmqtt:latest --id 1 --plugins-default-startups "rmqtt-cluster-raft" --node-grpc-addrs "1@172.17.0.3:5363" "2@172.17.0.4:5363" "3@172.17.0.5:5363" --raft-peer-addrs "1@172.17.0.3:6003" "2@172.17.0.4:6003" "3@172.17.0.5:6003"   

  docker run -d --name rmqtt2 -p 1885:1883 -p 8885:8883 -p 11885:11883 -p 6065:6060 -v /app/log/rmqtt/2:/var/log/rmqtt  rmqtt/rmqtt:latest --id 2 --plugins-default-startups "rmqtt-cluster-raft" --node-grpc-addrs "1@172.17.0.3:5363" "2@172.17.0.4:5363" "3@172.17.0.5:5363" --raft-peer-addrs "1@172.17.0.3:6003" "2@172.17.0.4:6003" "3@172.17.0.5:6003"   

  docker run -d --name rmqtt3 -p 1886:1883 -p 8886:8883 -p 11886:11883 -p 6066:6060 -v /app/log/rmqtt/3:/var/log/rmqtt  rmqtt/rmqtt:latest --id 3 --plugins-default-startups "rmqtt-cluster-raft" --node-grpc-addrs "1@172.17.0.3:5363" "2@172.17.0.4:5363" "3@172.17.0.5:5363" --raft-peer-addrs "1@172.17.0.3:6003" "2@172.17.0.4:6003" "3@172.17.0.5:6003"
```

节点IDs: 1, 2, 3; 节点IP Addrs: 172.17.0.3, 172.17.0.4, 172.17.0.5

#### 通过 docker-compose 创建静态集群

1. [下载配置模板](https://github.com/rmqtt/templates/blob/main/docker-compose-template/docker-compose-template.zip)

2. 启动 docker-compose 集群

```bash
docker-compose up -d
```

3. 查看集群

```bash
curl "http://127.0.0.1:6066/api/v1/health/check"
```

#### ZIP 压缩包安装 (Linux、MacOS、Windows)

需从 [GitHub Release](https://github.com/rmqtt/rmqtt/releases) 页面获取相应操作系统的ZIP压缩包。

- [单节点安装配置文档](./docs/zh_CN/install.md)

- [集群安装配置文档](./docs/zh_CN/install.md)

## 体验

[//]: # (- MQTT Broker：47.103.110.134:1883)

[//]: # (- Account: 无)

[//]: # (- HTTP APIs: http://47.103.110.134:6080/api/v1/)

## 测试

### 功能测试

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
   需要修改rmqtt-acl.toml配置，在第一行添加：["deny", "all", "subscribe", ["test/nosubscribe"]],

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
  * 需要修改rmqtt.toml配置，将max_keepalive改为60
* client_test5.py Test.test_subscribe_failure           [OK]
  * 需要修改rmqtt-acl.toml配置，在第一行添加：["deny", "all", "subscribe", ["test/nosubscribe"]],
同时修改client_test5.py的test_subscribe_failure()方法中的0x80为0x87。
因为rmqtt返回错误码是0x87, 而test_subscribe_failure要求返回0x80。
UnspecifiedError = 0x80, NotAuthorized = 0x87。


### 基准测试

#### 环境
| 项目          | 内容                                        |                                                                 |
|-------------|-------------------------------------------|-----------------------------------------------------------------|
| 操作系统        | x86_64 GNU/Linux                          | Rocky Linux 9.2 (Blue Onyx)                                     |
| CPU         | Intel(R) Xeon(R) CPU E5-2696 v3 @ 2.30GHz | 72(CPU(s)) = 18(Core(s)) * 2(Thread(s) per core) * 2(Socket(s)) |
| 内存          | DDR3/2333                                 | 128G                                                            |
| 磁盘          |                                           | 2T                                                              |
| 容器          | podman                                    | v4.4.1                                                          |
| 测试客户端       | docker.io/rmqtt/rmqtt-bench:latest        | v0.1.3                                                          |
| MQTT Broker | docker.io/rmqtt/rmqtt:latest              | v0.3.0                                                          |
| 其它          | 测试客户端和MQTT Broker同服         |                                                                 |

#### 连接并发性能
| 项目                | 单机              | Raft集群模式       |
|-------------------|-----------------|----------------|
| 并发客户端总数       | 100万            | 100万           |
| 连接握手速率        | (5500-7000)/秒   | (5000-7000)/秒  |

#### 消息吞吐性能
| 项目               | 单机                | Raft集群模式             |
|---------------------|------------------|----------------|
| 订阅客户端数量             | 100万             |   100万       |
| 发布客户端数量             | 40               |    40       |
| 消息吞吐速率              | 15万/秒            |   15.6万/秒   |

[基准测试详细内容，请参阅](./docs/zh_CN/benchmark-testing.md)





