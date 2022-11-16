# RMQTT Broker

[English](../en_US/install.md)  | 简体中文

## 安装

RMQTT 目前支持的操作系统:

- Linux
- macOS
- Windows Server

### ZIP 压缩包安装(Linux、MacOS、Windows)

需从 [GitHub Release](https://github.com/rmqtt/rmqtt/releases) 页面获取相应操作系统的二进制软件包。

1. 从[GitHub Release](https://github.com/rmqtt/rmqtt/releases) 下载zip包。

```bash
$ wget "https://github.com/rmqtt/rmqtt/releases/download/0.2.8/rmqtt-0.2.8-x86_64-unknown-linux-musl.zip"
```

2. 解压从[GitHub Release](https://github.com/rmqtt/rmqtt/releases) 下载的zip包。

```bash
$ unzip rmqtt-0.2.8-x86_64-unknown-linux-musl.zip -d /app/
```

3. 修改权限

```bash
$ cd /app/rmqtt
$ chmod +x bin/rmqttd
```

4. 启动服务

```bash
$ cd /app/rmqtt
$ sh start.sh
```

5. 查看服务

```bash
$ netstat -tlnp|grep 1883
tcp        0      0 0.0.0.0:1883            0.0.0.0:*               LISTEN      3312/./bin/rmqttd
tcp        0      0 0.0.0.0:11883           0.0.0.0:*               LISTEN      3312/./bin/rmqttd
```

### 创建static集群

#### 基于RAFT分布式一致性算法的集群

1. 准备三个服务节点，将压缩包解压到程序目录，比如：/app/rmqtt
2. 修改配置文件(rmqtt.toml)
    - 设置节点ID， node.id值设置为：1、2或3
    - 配置RPC服务端监听端口，rpc.server_addr = "0.0.0.0:5363"
    - 服务启动时同时启动"rmqtt-cluster-raft"插件

```bash
$ cd /app/rmqtt
$ vi etc/rmqtt.toml

##--------------------------------------------------------------------
## Node
##--------------------------------------------------------------------
#Node id
node.id = 1

##--------------------------------------------------------------------
## RPC
##--------------------------------------------------------------------
rpc.server_addr = "0.0.0.0:5363"

##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "./etc/plugins"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    "rmqtt-cluster-raft"
#    "rmqtt-auth-http",
#    "rmqtt-web-hook"
]
```

3. 修改插件配置

```bash
$ vi etc/plugins/rmqtt-cluster-raft.toml

##--------------------------------------------------------------------
## rmqtt-cluster-raft
##--------------------------------------------------------------------

#grpc message type
message_type = 198
#Node GRPC service address list
node_grpc_addrs = ["1@10.0.2.11:5363", "2@10.0.2.12:5363", "3@10.0.2.13:5363"]
#Raft peer address list
raft_peer_addrs = ["1@10.0.2.11:6363", "2@10.0.2.12:6363", "3@10.0.2.13:6363"]

```

4. 修改权限&启动服务

```bash
$ cd /app/rmqtt
$ chmod +x bin/rmqttd
$ sh start.sh
```

### 源码编译安装

#### 安装rust编译环境

以Centos7为例，如果编译环境已经存在跳过此过程。注意：工具链需要1.56及之后版本，1.59及之后版本如果报连接错误需要升级系统开发环境。

1. 安装 Rustup

   先打开 Rustup 的官网：https://rustup.rs ,然后根据提示下载或运行命令。

   Linux 下执行：

```bash
$ curl https://sh.rustup.rs -sSf | sh
```

执行source $HOME/.cargo/env 让环境变量生效

```bash
$ source $HOME/.cargo/env
```

2. 配置crate.io镜像

可以在$HOME/.cargo/下建立一个config文件，加入如下配置：

```bash
$ vi $HOME/.cargo/config

[source.crates-io]
registry = "https://github.com/rust-lang/crates.io-index"
replace-with = 'tuna'

[source.tuna]
registry = "https://mirrors.tuna.tsinghua.edu.cn/git/crates.io-index.git"

[source.ustc]
registry = "git://mirrors.ustc.edu.cn/crates.io-index"

[source.sjtu]
registry = "https://mirrors.sjtug.sjtu.edu.cn/git/crates.io-index"

[source.rustcc]
registry = "git://crates.rustcc.cn/crates.io-index"

[net]
git-fetch-with-cli = true
```

如果tuna也太慢可以使用sjtu或ustc替换重试

3. 安装openssl开发包

   确保已经安装了openssl的开发包，如果已经安装跳过

   For example, `libssl-dev` on Ubuntu or `openssl-devel` on Fedora.

CentOS:

```bash
$ yum install openssl-devel -y
```

Ubuntu:

```bash
$ apt install pkg-config -y
$ apt-get install libssl-dev -y
```

4. 安装protoc
   
   如果编译时报: protoc directory is not found，那么需要在以下位置下载并安装匹配安装包:

```bash
   https://github.com/protocolbuffers/protobuf/releases
```

##### 编译

1. 获取源码

```bash
$ git clone https://github.com/rmqtt/rmqtt.git
```

2. 切换到最近的 Tag

```bash
$ cd rmqtt
$ git checkout $(git describe --tags $(git rev-list --tags --max-count=1))
```

3. 构建

```bash
$ cargo build --release
```

##### 启动RMQTT Broker

1. 复制程序和配置文件

```bash
$ mkdir -p /app/rmqtt/bin && mkdir -p /app/rmqtt/etc/plugins
$ cp target/release/rmqttd /app/rmqtt/bin/
$ cp rmqtt.toml /app/rmqtt/etc/
$ cp rmqtt-plugins/*.toml /app/rmqtt/etc/plugins/
$ cp rmqtt-bin/rmqtt.pem  /app/rmqtt/etc/
$ cp rmqtt-bin/rmqtt.key  /app/rmqtt/etc/
```

2. 修改配置(rmqtt.toml)

- 将plugins.dir = "rmqtt-plugins/" 改为 plugins.dir = "/app/rmqtt/etc/plugins"
- 根据需要打开插件，如果启用插件，可在/etc/plugins/下修改插件配置
- 如果需要启动TLS，可修改listener.tls.external配置

```bash
vi /app/rmqtt/etc/rmqtt.toml

##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "/app/rmqtt/etc/plugins"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #    "rmqtt-cluster-raft"
    #    "rmqtt-cluster-broadcast",
    #    "rmqtt-auth-http",
    #    "rmqtt-web-hook"
]



##--------------------------------------------------------------------
## MQTT/TLS - External TLS Listener for MQTT Protocol
listener.tls.external.addr = "0.0.0.0:8883"
listener.tls.external.cert = "/app/rmqtt/etc/rmqtt.pem"
listener.tls.external.key = "/app/rmqtt/etc/rmqtt.key"
```

3. 启动服务

```bash
$ cd /app/rmqtt
./bin/rmqttd -f "./etc/rmqtt.toml"
```















