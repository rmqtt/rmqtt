[English](../en_US/development/getting-started.md) | [**简体中文**](getting-started.md)

# RMQTT 开发入门

本文档指导你搭建开发环境、编译 RMQTT、运行测试并了解开发工作流。

---

## 前置要求

### 必需

- **Rust** 1.89.0+（通过 [rustup](https://rustup.rs/) 安装）
- **protoc**（protobuf 编译器）— 构建 tonic/prost 依赖必需
- **Git**（用于克隆和管理版本）

### 平台特定设置

**Linux (Ubuntu/Debian)**：
```bash
sudo apt update
sudo apt install build-essential protobuf-compiler cmake pkg-config libssl-dev curl-dev zlib1g-dev
```

**macOS**：
```bash
brew install protobuf cmake pkg-config openssl
```

**Windows**：
1. 安装 [protoc](https://github.com/protocolbuffers/protobuf/releases) — 下载 `protoc-{version}-win64.zip`，解压到 `C:\protoc`，加入 PATH
2. 安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022)

### 验证安装

```bash
rustc --version          # 应为 1.89.0+
cargo --version
protoc --version         # 应为 3.x+
```

---

## 克隆与构建

```bash
git clone https://github.com/rmqtt/rmqtt.git
cd rmqtt

# Debug 构建（快速编译，用于开发）
cargo build

# Release 构建（优化，用于测试和生产）
cargo build --release

# 构建特定子 crate
cargo build -p rmqtt-codec
cargo build -p rmqttd
```

生产二进制文件位于 `target/release/rmqttd`（Windows 上为 `rmqttd.exe`）。

---

## 开发工作流

### 编码 → 构建 → 测试 循环

```bash
cargo check              # 快速验证（不生成二进制）
cargo build -p rmqtt     # 构建特定 crate
cargo test -p rmqtt-codec # 运行 crate 的单元测试
```

### 代码检查

```bash
# 格式化
cargo fmt --all

# Lint（必须零警告）
cargo clippy --all-targets
```

### 运行完整测试套件

```bash
# 构建 release 二进制（测试框架需要）
cargo build --release

# 运行所有单元测试
cargo test

# 运行集成测试
cargo build -p rmqtt-test --release
./target/release/mqtt_harness --workspace .
```

### 手动测试

```bash
# 启动开发模式 Broker
cargo run -p rmqttd

# 或使用自定义配置
cargo run -p rmqttd -- -f my-config.toml

# 使用 mosquitto 客户端测试
mosquitto_sub -h 127.0.0.1 -p 1883 -t "test/#" -v
mosquitto_pub -h 127.0.0.1 -p 1883 -t "test/topic" -m "hello"
```

---

## 使用 Feature 标志

核心库（`rmqtt`）有 15 个 feature 标志：

```bash
# 最小插件开发
cargo build -p rmqtt --features "plugin"

# 完整功能（rmqttd 使用的）
cargo build -p rmqtt --features "full"
```

---

## 插件开发

创建新插件：
1. 在 `rmqtt-plugins/` 下创建新 crate
2. 添加到 `rmqtt-plugins/Cargo.toml` 作为可选依赖
3. 在 `rmqtt-plugins/src/lib.rs` 中添加 feature 标志和 re-export
4. 遵循标准 `Plugin trait` 模式

更详细的贡献指南请参阅 [CONTRIBUTING.md](../../CONTRIBUTING.md)。

## 许可证

MIT OR Apache-2.0
