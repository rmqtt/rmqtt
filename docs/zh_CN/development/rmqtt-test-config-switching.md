# rmqtt-test 按用例自动切换 Broker 配置

> 本文档记录 rmqtt-test（`mqtt_harness`）单机测试框架的配置管理机制调整：
> **每个用例 / 每组用例可以使用自己的 broker 配置文件**，且测试配置全部
> **自包含**于 `rmqtt-test/configs/`，不再依赖仓库根的 `rmqtt.toml` 与
> `rmqtt-plugins/*.toml`。

---

## 1. 背景与问题

调整前，单机测试（非 `--no-broker` 模式）的行为：

- `mqtt_harness` 启动 broker 时，仅当命令行给了 `--config` 才传 `-f <config>`，
  否则 `rmqttd` 直接读取**仓库根 `rmqtt.toml`**；
- 整个 harness 进程生命周期内只有**一套** broker 配置，配置只能"整批"指定。

这导致两类用例无法在同一次默认全量运行中共存：

| 用例 | 需要的配置 | 默认配置下的表现 |
|------|-----------|-----------------|
| `will_retain_rejected_when_retain_unavailable_v5`（issue #457） | 不加载 retainer 插件（`Retain Available = 0`） | 只能走 `NotApplicable` 分支被跳过 |
| `qos2_pubrel_resume_collision` | 加载 message-storage 插件 | 无法复现存储消息重载场景 |

**目标**：让用例声明自己需要的配置，框架在合适时机自动切换 broker 配置；
同时让测试配置自包含，与开发环境配置彻底解耦。

## 2. 方案总览

- **纯 suite 级切换**：调度器只在 **suite 边界**重启 broker 切换配置，不做用例级运行时切换；
- **构建期自动拆分**：声明了同一非默认配置的用例，在构建套件时自动拆分为独立的
  `{suite}@{config}` 子套件，每个子套件内配置唯一；
- **配置自包含**：默认配置与各测试配置都放在 `rmqtt-test/configs/<name>/`，
  主配置 + 自身 `plugins/` 子目录，不依赖仓库根。

## 3. 配置自包含（`configs/` 目录）

```
rmqtt-test/configs/
  default/                  # 默认配置（未指定 --config 时使用）
    rmqtt.toml              #   以仓库根 rmqtt.toml 为蓝本
    plugins/                #   全部插件配置模板（26 个）
  retain-disabled/          # 不加载 retainer 插件（Retain Available = 0）
    rmqtt.toml
    plugins/
  pubrel-collision/         # 加载 message-storage（PUBREL 冲突复现）
    rmqtt.toml
    plugins/
  tcp-keepalive/            # TCP keepalive 短探测参数（issue #465 参数生效验证）
    rmqtt.toml              #   tcp_keepalive = { idle = "1s", interval = "1s", probes = 2 }
    plugins/
  pubrel-collision-cluster/ # 双节点集群（手动启动，1884/1885 MQTT）—— 不受本机制影响
```

### 3.1 `configs/default/rmqtt.toml` 与仓库根配置的差异

| 配置项 | 仓库根值 | default 值 | 原因 |
|---|---|---|---|
| `plugins.dir` | `"rmqtt-plugins/"` | `"rmqtt-test/configs/default/plugins/"` | 自包含 |
| `listener.tcp.external.addr` | `"0.0.0.0:1883"` | `"127.0.0.1:1883"` | 匹配 harness 默认 `--addr` |
| `log.to` / `log.dir` | `"both"` / `/var/log/rmqtt` | `"console"` / `"."` | 测试环境避免写系统目录 |
| `plugins.default_startups` | retainer / shared-subscription / http-api | **保持不变** | 默认插件集与仓库根一致 |

**保留全部 TCP/TLS/WS/WSS/QUIC 监听**（用户明确要求，为后续 TLS/WS/QUIC 专项测试预留）。

### 3.2 为什么 plugins/ 要放全部插件配置（重要）

`rmqtt-http-api` 插件在 Cargo.toml metadata 中 `default_startup = true`（与
`rmqtt-acl`、`rmqtt-counter` 一样会自动启动），且**强制要求存在配置文件**——
缺失时 `rmqttd` 注册插件直接失败（release 版 panic，debug 版卡死）。而
`rmqtt-acl` / `rmqtt-counter` 缺配置时仅 WARN 并使用默认值。

因此每个 `configs/<name>/plugins/` 目录内**完整拷贝** `rmqtt-plugins/*.toml`
（26 个文件）。多余文件不会被加载，但保证任何自动启动插件都能找到配置。

## 4. 机制设计

### 4.1 用例级配置声明（仅分组依据）

`TestCase` trait 新增 `broker_config()`：

```rust
fn broker_config(&self) -> Option<PathBuf> { None }
```

- 返回 `None` → 使用 harness 默认配置（`--config` 或 `configs/default/rmqtt.toml`）；
- 返回路径 → 该用例声明需要特定配置，仅作为**构建期分组依据**，调度器不做用例级切换。

辅助函数 `tests::config_path(name)` 生成 `configs/<name>/rmqtt.toml` 的绝对路径
（基于 `CARGO_MANIFEST_DIR`，与进程工作目录无关）。

### 4.2 构建期自动拆分

`TestSuite` 新增 `config: Option<PathBuf>` 字段；`split_suites_by_config()`
（`src/framework/suite.rs`）在 `build_suites()` 之后统一执行：

- suite 已有显式 `config`（如集群套件）→ 不拆分；
- 否则按用例 `broker_config()` 分组（保持组内原始相对顺序）：
  - 默认组保留原名（如 `functional_v5`），`config = 默认配置路径`；
  - 特殊组生成 `{suite}@{config名}`（如 `functional_v5@retain-disabled`），
    `config = 该配置路径`。

拆分后**每个 suite 的 `config` 恒有值**，调度器只需在 suite 边界判断。

### 4.3 Broker 进程配置切换

`src/broker/lifecycle.rs`：

```rust
pub fn set_config(&mut self, config: Option<PathBuf>);            // 仅更新，不重启
pub fn config_path(&self) -> Option<&PathBuf>;                    // 当前生效配置
pub fn restart_with_config(&mut self, config: Option<PathBuf>);   // stop → 换配置 → start
```

`BrokerProcess::new(workspace)` 默认 `config_path` 指向
`<workspace>/rmqtt-test/configs/default/rmqtt.toml`（**始终显式 `-f` 启动**，
不再存在"无配置"路径）。

### 4.4 TestContext 幂等切换

```rust
pub fn ensure_broker_config(&self, target: &Path) -> Result<(), anyhow::Error>
```

比较 `BrokerProcess::config_path()` 与 `target`，相同则跳过重启（幂等）；
不同则 `restart_with_config` 并等待健康检查。`--no-broker` 模式下直接返回 `Ok`
（由调度器负责告警）。

### 4.5 调度器：suite 边界切换

`src/framework/scheduler.rs` 的 `run()`：每个 suite 执行前，若
`suite.config` 与当前生效配置不一致则切换：

- 切换失败 → 该 suite 记为 `Error`（critical），跳过执行；
- `--no-broker` → 忽略所有配置要求并 warn 一次。

### 4.6 main.rs 流程与 CLI 匹配

流程：`resolve_workspace()` → 解析默认配置 → 启动 broker → `build_suites()`
→ `split_suites_by_config()` → `filter_suites()` → 运行。

- **workspace 解析**：`--workspace` 显式值 → cwd 探测（有 `rmqtt.toml` 且
  `rmqtt-test/configs/`）→ `CARGO_MANIFEST_DIR` 父目录（编译期仓库根）；
- **默认配置**：`--config` 或 `<workspace>/rmqtt-test/configs/default/rmqtt.toml`，
  不存在则报错退出；启动时打印生效配置路径；
- **`--suites` 前缀匹配**（`should_run` 双向匹配 + `filter_suites` 单向前缀）：
  - `functional_v5` → 同时命中 `functional_v5` 与 `functional_v5@retain-disabled` 等子套件；
  - `functional_v5@retain-disabled` → 只跑该子套件（也会注册原套件）；
  - `functional_v5_cluster` → 仅显式指定时运行，默认全量排除（行为保持）。

## 5. 使用方式

```bash
# 默认全量运行（默认配置 = configs/default，自动切换特殊子套件）
./target/release/mqtt_harness --workspace .

# 显式指定整套配置
./target/release/mqtt_harness --workspace . --config rmqtt-test/configs/retain-disabled/rmqtt.toml

# 只跑某个配置子套件
./target/release/mqtt_harness --workspace . --suites functional_v5@retain-disabled

# 只跑默认配置组（不跑 @ 子套件）
./target/release/mqtt_harness --workspace . --suites functional_v5
```

运行日志中可见拆分与切换过程：

```
split suite 'functional_v5' -> 'functional_v5' (61 tests, config: .../configs/default/rmqtt.toml)
split suite 'functional_v5' -> 'functional_v5@retain-disabled' (1 tests, ...)
split suite 'functional_v5' -> 'functional_v5@pubrel-collision' (1 tests, ...)
Running suite: functional_v5 (61 tests)
...
switching broker config: ...default/rmqtt.toml -> ...retain-disabled/rmqtt.toml
Broker is healthy at 127.0.0.1:1883
Running suite: functional_v5@retain-disabled (1 tests)
```

## 6. 新增一个特殊配置目录的步骤

1. 创建 `rmqtt-test/configs/<name>/rmqtt.toml`（以 `default/rmqtt.toml` 为蓝本，
   修改 `plugins.default_startups` 与所需参数；监听端口保持 `127.0.0.1:1883`）；
2. 创建 `rmqtt-test/configs/<name>/plugins/`，拷贝 `rmqtt-plugins/*.toml`（26 个）；
3. 用例实现 `broker_config()` 返回 `Some(crate::tests::config_path("<name>"))`；
4. 无需改动调度器——拆分与切换自动生效。

## 7. 约束与注意事项

- **端口一致性**：参与自动切换的配置，`listener.tcp.external.addr` 必须与
  harness 的 `--addr`（默认 `127.0.0.1:1883`）一致，否则健康检查无法通过；
- **重启成本**：一次配置切换 = 一次 broker 重启（约 1~2s + 健康检查轮询）。
  默认全量运行只产生特殊配置组各 1 次切换，且每组只切一次、无需切回；
- **状态隔离**：切换必然重启 → broker 内存状态清空，天然隔离；跨配置的用例
  不应有依赖关系（DAG 依赖只在同一配置组内成立）；
- **并行 suite**：配置切换只发生在 suite 边界，parallel suite 内不切换；
- **`--no-broker` 模式**：无法切换配置，配置声明被忽略并告警一次；
- **工作目录**：`plugins.dir` 是相对进程 cwd 解析的，harness 与 rmqttd 都
  应从仓库根运行（`--workspace .`）。

## 8. 涉及文件

| 文件 | 改动 |
|---|---|
| `rmqtt-test/configs/default/rmqtt.toml` + `plugins/*` | 新增：自包含默认配置 |
| `rmqtt-test/configs/retain-disabled/`、`pubrel-collision/` | `plugins.dir` 指向自身 plugins/ + 补齐插件配置 |
| `src/broker/lifecycle.rs` | +`set_config` / `restart_with_config` / `config_path`；`new()` 默认配置 |
| `src/framework/testcase.rs` | trait +`broker_config()` |
| `src/framework/suite.rs` | +`config` 字段；+`split_suites_by_config()` |
| `src/framework/context.rs` | +`ensure_broker_config()`（幂等） |
| `src/framework/scheduler.rs` | suite 边界配置切换；失败记 Error；`--no-broker` 忽略 |
| `src/main.rs` | workspace 解析、默认配置来源、build→split→filter、前缀匹配、失败路径 `drop(ctx)` 防泄漏 |
| `src/tests/mod.rs` | +`config_path()` 辅助 |
| `src/tests/functional/retain_unavailable_v5.rs` | 声明 `retain-disabled` |
| `src/tests/functional/qos2_pubrel_resume_collision.rs` | 声明 `pubrel-collision` |

## 9. 验证结果

- `cargo build -p rmqtt-test` 零警告；`cargo test -p rmqtt-test`（21 个 CLI 测试）全过；
- release 全量 `functional_v5`：**63/63 通过**（61 默认组 + 1@retain-disabled +
  1@pubrel-collision），两次配置切换成功，`will_retain_rejected_when_retain_unavailable_v5`
  真正执行（不再 skip）。

## 10. 过程中发现并修复的问题

1. **`rmqtt-http-api` 插件强制要求配置文件**（`default_startup=true` 且缺失即
   启动失败）→ 自包含目录必须全量拷贝插件配置（见 3.2）；
2. **失败路径进程泄漏**：`main.rs` 原用 `std::process::exit(1)` 直接退出，
   **不执行 Drop**，导致 broker 子进程残留并占用端口（1883/6060/5363），
   还会干扰后续运行的健康检查（误连旧进程造成"假失败"）。修复：`exit` 前
   显式 `drop(ctx)`；
3. **验证环境坑（Windows）**：Git Bash 中 `taskkill //F` 双斜杠转义失效，
   杀不掉残留进程，需用 PowerShell `Stop-Process -Name rmqttd -Force`。
