# RMQTT Broker

[![GitHub Release](https://img.shields.io/github/release/rmqtt/rmqtt?color=brightgreen)](https://github.com/rmqtt/rmqtt/releases)
<a href="https://blog.rust-lang.org/2022/09/22/Rust-1.64.0.html"><img alt="Rust Version" src="https://img.shields.io/badge/rust-1.64%2B-blue" /></a>

English | [简体中文](./README-CN.md)

*RMQTT* broker is a fully open source, highly scalable, highly available distributed MQTT messaging broker for IoT, M2M
and mobile applications that can handle millions of concurrent clients on a single service node.

## Features

- 100% Rust safe code;
- Based on [tokio](https://crates.io/crates/tokio), [ntex](https://crates.io/crates/ntex)
  , [ntex-mqtt](https://crates.io/crates/ntex-mqtt);
- MQTT v3.1, v3.1.1 and v5.0 protocols support;
    - QoS0, QoS1, QoS2 message support;
    - Offline message support;
    - Retained message support;
    - Last Will message support;
- [Built-in AUTH/ACL](./docs/en_US/acl.md);
- [HTTP AUTH/ACL](./docs/en_US/auth-http.md);
- [WebHook](./docs/en_US/web-hook.md);
- [HTTP APIs](./docs/en_US/http-api.md);
- Distributed cluster;
- Hooks;
- TLS support;
- WebSocket support;
- WebSocket-TLS support;
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

1. [Download docker-compose configuration template](https://github.com/rmqtt/templates/blob/main/docker-compose-template/docker-compose-template.zip)

2. Start docker-compose cluster

```bash
docker-compose up -d
```

3. View cluster

```bash
curl "http://127.0.0.1:6066/api/v1/brokers"
```

#### Installing via ZIP Binary Package (Linux、MacOS、Windows)

Get the binary package of the corresponding OS from [RMQTT Download](https://github.com/rmqtt/rmqtt/releases) page.

- [Single Node Install](./docs/en_US/install.md)

- [Multi Node Install](./docs/en_US/install.md)

## Experience

- MQTT Borker: 121.4.74.58:1883
- Account:
- HTTP APIs: http://121.4.74.58:6060/api/v1/

