# RMQTT Broker

[![GitHub Release](https://img.shields.io/github/release/rmqtt/rmqtt?color=brightgreen)](https://github.com/rmqtt/rmqtt/releases)
<a href="https://blog.rust-lang.org/2022/02/24/Rust-1.59.0.html"><img alt="Rust Version" src="https://img.shields.io/badge/rust-1.59%2B-blue" /></a>

English | [简体中文](./README-CN.md)

*RMQTT* broker is a fully open source, highly scalable, highly available distributed MQTT messaging broker for IoT, M2M
and mobile applications that can handle millions of concurrent clients on a single service node.

## Features

- 100% Rust safe code;
- Based on [tokio](https://crates.io/crates/tokio), [ntex](https://crates.io/crates/ntex), [ntex-mqtt](https://crates.io/crates/ntex-mqtt);
- MQTT v3.1, v3.1.1 and v5.0 protocols support;
  - QoS0, QoS1, QoS2 message support;
  - Offline message support;
  - Retained message support;
  - Last Will message support;
- Built-in ACL;
- HTTP ACL;
- WebHook;
- [HTTP APIs](./docs/en_US/http-api.md);
- Distributed cluster;
- Hooks;
- TLS support;
- Shared subscription($share/{group}/topic);
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

#### Installing via RMQTT Docker Image

```
```

#### Installing via ZIP Binary Package (Linux、MacOS、Windows)

Get the binary package of the corresponding OS from [RMQTT Download](https://github.com/rmqtt/rmqtt/releases) page.

- [Single Node Install](./docs/en_US/install.md)

- [Multi Node Install](./docs/en_US/install.md)

## Experience

- MQTT Borker: 121.4.74.58:1883
- Account: 
- HTTP APIs: http://121.4.74.58:6060/api/v1/

