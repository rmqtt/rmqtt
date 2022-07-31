# RMQTT Broker

[![GitHub Release](https://img.shields.io/github/release/rmqtt/rmqtt?color=brightgreen)](https://github.com/rmqtt/rmqtt/releases)

[English](./README.md)  | 简体中文

*RMQTT* 是一款完全开源，高度可伸缩，高可用的分布式 MQTT 消息服务器，适用于 IoT、M2M 和移动应用程序，可以在单个服务节点上处理百万级别的并发客户端。

## 功能特色

- 100% Rust安全编码;
- 基于 [tokio](https://crates.io/crates/tokio), [ntex](https://crates.io/crates/ntex), [ntex-mqtt](https://crates.io/crates/ntex-mqtt) 开发;
- 支持MQTT v3.1,v3.1.1 及 v5.0协议;
  - QoS0, QoS1, QoS2 消息支持;
  - 离线消息支持;
  - Retained 消息支持;
  - Last Will 消息支持;
- 内置 ACL;
- HTTP ACL;
- WebHook;
- [HTTP API](./docs/zh_CN/http-api.md);
- 分布式集群;
- 钩子(Hooks);
- TLS支持;
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

#### RMQTT Docker 镜像安装

#### ZIP 压缩包安装 (Linux、MacOS、Windows)

需从 [GitHub Release](https://github.com/rmqtt/rmqtt/releases) 页面获取相应操作系统的ZIP压缩包。

- [单节点安装配置文档](./docs/install-cn.md)

- [集群安装配置文档](./docs/install-cn.md)

## 体验

- MQTT Borker：121.4.74.58:1883
- Account: 无
- HTTP APIs: http://121.4.74.58:6060/api/v1/





