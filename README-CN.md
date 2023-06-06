# RMQTT Broker

[![GitHub Release](https://img.shields.io/github/release/rmqtt/rmqtt?color=brightgreen)](https://github.com/rmqtt/rmqtt/releases)
<a href="https://blog.rust-lang.org/2022/09/22/Rust-1.64.0.html"><img alt="Rust Version" src="https://img.shields.io/badge/rust-1.64%2B-blue" /></a>

[English](./README.md)  | 简体中文

*RMQTT* 是一款完全开源，高度可伸缩，高可用的分布式 MQTT 消息服务器，适用于 IoT、M2M 和移动应用程序，可以在单个服务节点上处理百万级别的并发客户端。

## 功能特色

- 100% Rust安全编码;
- 基于 [tokio](https://crates.io/crates/tokio), [ntex](https://crates.io/crates/ntex)
  , [ntex-mqtt](https://crates.io/crates/ntex-mqtt) 开发;
- 支持MQTT v3.1,v3.1.1 及 v5.0协议;
    - QoS0, QoS1, QoS2 消息支持;
    - 离线消息支持;
    - Retained 消息支持;
    - Last Will 消息支持;
- [内置 ACL](./docs/zh_CN/acl.md);
- HTTP ACL;
- WebHook;
- [HTTP APIs](./docs/zh_CN/http-api.md);
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

1. [下载配置模板](./examples/docker-compose.zip)

2. 启动 docker-compose 集群

```bash
docker-compose up -d
```

3. 查看集群

```bash
curl "http://127.0.0.1:6066/api/v1/brokers"
```

#### ZIP 压缩包安装 (Linux、MacOS、Windows)

需从 [GitHub Release](https://github.com/rmqtt/rmqtt/releases) 页面获取相应操作系统的ZIP压缩包。

- [单节点安装配置文档](./docs/zh_CN/install.md)

- [集群安装配置文档](./docs/zh_CN/install.md)

## 体验

- MQTT Borker：121.4.74.58:1883
- Account: 无
- HTTP APIs: http://121.4.74.58:6060/api/v1/





