[English](CONTRIBUTING.md) | [**简体中文**](CONTRIBUTING-CN.md)

# 参与 RMQTT 贡献

感谢你考虑为 RMQTT 贡献代码！本文档概述了开发工作流、代码规范和流程。

## 目录

- [行为准则](#行为准则)
- [报告问题](#报告问题)
- [开发环境](#开发环境)
- [代码风格](#代码风格)
- [测试](#测试)
- [文档](#文档)
- [Pull Request 流程](#pull-request-流程)
- [许可证](#许可证)

## 行为准则

本项目致力于为所有人提供友好、包容的体验。我们期望所有贡献者：

- 保持尊重和体谅
- 虚心接受建设性批评
- 以社区最佳利益为重
- 对其他社区成员展现同理心

## 报告问题

提交 Issue 之前：

1. **搜索现有 Issue** — 检查问题是否已被报告
2. **使用清晰描述性标题** — 一句话概括问题
3. **包含复现步骤** — 复现所需的最小代码/配置
4. **注明环境信息** — 操作系统、Rust 版本（`rustc --version`）、RMQTT 版本

### Bug 报告模板

```markdown
## 描述
[清晰描述 Bug]

## 复现步骤
1. 启动 Broker: `...`
2. 连接客户端: `...`
3. 发布到: `...`

## 预期行为
[应该发生什么]

## 实际行为
[实际发生了什么]

## 环境
- OS: [如 Ubuntu 24.04]
- Rust: [如 1.89.0]
- RMQTT: [如 0.22.0]
```

### 功能请求模板

```markdown
## 问题
[这个功能解决了什么问题？]

## 建议方案
[你希望这个功能如何工作？]

## 替代方案
[其他解决方式？]
```

## 开发环境

### 前置要求

- Rust 1.89.0+（通过 [rustup](https://rustup.rs/) 安装）
- `protoc`（protobuf 编译器）— 构建 tonic/prost 必需
  - Linux: `apt install protobuf-compiler`
  - macOS: `brew install protobuf`
  - Windows: 从 [protobuf releases](https://github.com/protocolbuffers/protobuf/releases) 下载

### 构建

```bash
# 克隆仓库
git clone https://github.com/rmqtt/rmqtt.git
cd rmqtt

# 构建所有 crate
cargo build

# Release 构建（所有功能）
cargo build --release

# 构建特定 crate
cargo build -p rmqttd
```

### 代码生成

部分代码在构建时自动生成：

```bash
# 插件注册代码（从 Cargo.toml 元数据自动生成）
# 由 rmqtt-bin/build.rs 自动完成 — 无需手动操作
```

## 代码风格

### 格式化

```bash
# 格式化所有代码
cargo fmt --all
```

我们使用 `rustfmt` 默认配置。所有代码在提交前必须格式化。

### Lint

```bash
# 对所有 target 运行 clippy
cargo clippy --all-targets
```

项目始终保持 **零 clippy 警告**。新代码不应引入新警告。

### 安全性

全项目强制 `#![deny(unsafe_code)]`。生产代码禁止包含：

- `unsafe` 块
- `unwrap()` 或 `expect()`（仅测试代码可用）
- 生产路径中的 `panic!` / `todo!` / `unreachable!`
- `#[allow(unused_*)]`（除非在条件编译标志后）

### 提交信息

我们遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范：

```
<type>(<scope>): <description>
```

**类型**：`feat`、`fix`、`docs`、`style`、`refactor`、`test`、`chore`、`deps`、`ci`

**范围**（示例）：`core`、`net`、`codec`、`conf`、`test`、`plugin/acl`、`plugin/raft`、`cli`

**示例**：
```
feat(net): add cert_subject_dn_as_username option for TLS listeners
fix(http-api): add startup synchronization and improve reload handling
deps: upgrade prometheus from 0.13 to 0.14
docs(test): update CLI usage examples for rmqtt-test
```

## 测试

### 运行测试

```bash
# 运行所有单元测试
cargo test

# 运行特定 crate 的测试
cargo test -p rmqtt-codec

# 使用测试框架运行集成测试（需要 release 构建）
cargo build -p rmqttd --release
cargo build -p rmqtt-test --release
./target/release/mqtt_harness --workspace .
```

### 测试套件

项目有两层测试：

1. **单元测试** — 每个 crate 中的标准 `#[cfg(test)]` 模块
2. **集成测试** — `rmqtt-test` 框架提供五个套件：

| 套件 | 说明 |
|-------|------|
| `functional_v3` | MQTT 3.1 基本操作 |
| `functional_v311` | MQTT 3.1.1 协议合规（10 个用例） |
| `functional_v5` | MQTT 5.0 协议合规（5 个用例） |
| `stress` | 连接/发布/扇出负载测试 |
| `chaos` | Broker 重启、连接风暴、慢消费者 |

### 互操作性测试

RMQTT 通过了 [paho.mqtt.testing](https://github.com/eclipse/paho.mqtt.testing) 测试套件：

```bash
# MQTT v3.1.1（11 个测试）
python client_test.py

# MQTT v5.0（24 个测试）
python client_test5.py
```

### 性能基准测试

```bash
# 构建 release
cargo build --release

# 运行基准测试配置
# 详见 docs/zh_CN/benchmark-testing.md
```

## 文档

所有文档遵循以下标准：

### 代码文档

- 所有公开 API 必须包含 doc 注释（`///`）
- `lib.rs` 中包含 crate 级别的文档，说明用途和架构
- 文档注释中的示例应可运行（必要时使用 `rust,no_run`）
- 使用 `#[cfg(feature = "...")]` 标注按功能开关的项

### 项目文档

位于 `docs/` 目录，支持双语：

| 目录 | 语言 |
|-----------|-------|
| `docs/en_US/` | 英文 |
| `docs/zh_CN/` | 简体中文 |
| `docs/imgs/` | 共享图片 |

添加新功能时，需同时创建或更新英文和中文文档。

### README 文件

每个 crate 必须包含：
- `README.md` — 英文版，包含徽章、概述、用法、API 参考
- `README-CN.md` — 简体中文版，语义等价

## Pull Request 流程

1. **从最新的 `master` 创建功能分支**：

   ```bash
   git checkout master
   git pull
   git checkout -b feat/my-feature
   ```

2. **按照上述代码规范进行修改**。

3. **验证修改**：

   ```bash
   cargo fmt --all
   cargo clippy --all-targets
   cargo test
   cargo build --release
   ```

4. **提交修改**，使用清晰的提交信息：

   ```bash
   git commit -m "feat(scope): concise description"
   ```

5. **推送并创建 PR**：

   ```bash
   git push origin feat/my-feature
   ```

   然后在 GitHub 上向 `master` 分支发起 Pull Request。

6. **PR 检查清单**：
   - [ ] 代码编译无警告
   - [ ] `cargo clippy --all-targets` 通过
   - [ ] `cargo test` 通过
   - [ ] 新特性包含测试
   - [ ] 公开 API 有 doc 注释
   - [ ] 文档已更新（英文 + 中文）
   - [ ] 提交信息遵循 conventional 格式

### PR 审查标准

维护者将从以下方面审查你的 PR：

- **正确性** — 代码是否实现其声称的功能？
- **安全性** — 无不安全代码、无 panic 路径
- **性能** — 正确使用异步、锁和数据结构
- **一致性** — 遵循代码库现有模式
- **文档** — 清晰的 doc 注释和文档更新

## 许可证

向 RMQTT 贡献代码即表示你同意你的贡献将按照 [MIT](LICENSE-MIT) 或 [Apache 2.0](LICENSE-APACHE) 许可证（由你选择）进行许可。
