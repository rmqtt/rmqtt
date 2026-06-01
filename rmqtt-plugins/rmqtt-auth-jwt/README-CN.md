[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-auth-jwt

[![crates.io](https://img.shields.io/crates/v/rmqtt-auth-jwt.svg)](https://crates.io/crates/rmqtt-auth-jwt)

RMQTT 的 JWT 认证插件。验证 JSON Web Token 进行客户端认证。

## 概述

从客户端的密码或用户名字段中提取并验证 JWT 令牌。支持基于 HMAC 的加密（HS256/HS384/HS512）和公钥加密（RS256/RS384/RS512、ES256/ES384/ES512）。提供声明验证功能，包括 `exp`、`nbf`、`sub`、`iss`、`aud` 和扩展的自定义声明。

### 认证流程

1. 客户端连接并在密码或用户名字段中提供 JWT
2. 插件从配置的字段（`from`）中提取 JWT
3. 根据 `encrypt` 设置：
   - **hmac-based**：使用 HMAC 密钥（`hmac_secret`）解码，可选择 Base64 解码（`hmac_base64`）
   - **public-key**：使用 RSA/ECDSA 公钥文件（`public_key`）解码
4. 验证标准声明（`exp`、`nbf`、`sub`、`iss`、`aud`）（如已配置）
5. 验证扩展的自定义声明与实际客户端属性是否匹配
6. 验证通过 → 认证成功；否则 → 认证失败

### 支持的算法

| 类别 | 算法 |
|------|------|
| HMAC | HS256, HS384, HS512 |
| RSA | RS256, RS384, RS512 |
| ECDSA | ES256, ES384, ES512 |

## 使用

在 `Cargo.toml` 中添加依赖：

```toml
rmqtt-auth-jwt = "0.22"
```

在代理启动代码中注册插件：

```rust
rmqtt_auth_jwt::register(&scx, true, false).await?;
```

## 配置

配置文件：`rmqtt-auth-jwt.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `disconnect_if_pub_rejected` | Boolean | `true` | 发布被拒绝时是否断开客户端 |
| `disconnect_if_expiry` | Boolean | `false` | 过期后是否断开客户端 |
| `from` | String | `"password"` | JWT 来源：`"username"` 或 `"password"` |
| `encrypt` | String | `"hmac-based"` | 加密方式：`"hmac-based"` 或 `"public-key"` |
| `hmac_secret` | String | `"rmqttsecret"` | HMAC 哈希密钥 |
| `hmac_base64` | Boolean | `false` | HMAC 密钥是否经过 Base64 编码 |
| `public_key` | String (文件) | *(无)* | RSA 或 ECDSA 公钥文件路径（`encrypt = "public-key"` 时必须） |
| `validate_claims.exp` | Boolean | `true` | 验证令牌过期时间（`exp` 声明） |
| `validate_claims.nbf` | Boolean | `false` | 验证不早于时间（`nbf` 声明） |
| `validate_claims.sub` | String | *(无)* | 期望的主题（`sub` 声明）值 |
| `validate_claims.iss` | String[] | *(无)* | 期望的签发者（`iss` 声明） |
| `validate_claims.aud` | String[] | *(无)* | 期望的受众（`aud` 声明） |
| `validate_claims.clientid` | String | *(无)* | 期望的客户端 ID（支持 `${clientid}` 占位符） |
| `validate_claims.username` | String | *(无)* | 期望的用户名（支持 `${username}` 占位符） |
| `validate_claims.ipaddr` | String | *(无)* | 期望的 IP 地址（支持 `${ipaddr}` 占位符） |
| `validate_claims.protocol` | String | *(无)* | 期望的协议版本（支持 `${protocol}` 占位符） |

### 声明验证占位符

配置扩展声明（`clientid`、`username`、`ipaddr`、`protocol`）时，可使用以下占位符匹配动态客户端属性：

| 占位符 | 描述 |
|--------|------|
| `${username}` | 客户端用户名 |
| `${clientid}` | 客户端 ID |
| `${ipaddr}` | 客户端 IP 地址 |
| `${protocol}` | MQTT 协议版本：`3` = 3.1, `4` = 3.1.1, `5` = 5.0 |

当声明设置为 `${clientid}` 这样的占位符时，表示 JWT 中的 `clientid` 声明必须与实际客户端的客户端 ID 匹配。

## 配置示例

### HMAC 方式（默认）

```toml
from = "password"
encrypt = "hmac-based"
hmac_secret = "rmqttsecret"
hmac_base64 = false

validate_claims.exp = true
validate_claims.sub = "user@rmqtt.com"
validate_claims.iss = ["https://rmqtt.com"]
validate_claims.clientid = "${clientid}"
```

### 公钥方式（RSA/ECDSA）

```toml
from = "password"
encrypt = "public-key"
public_key = "./rmqtt-bin/jwt_public_key_rsa.pem"

validate_claims.exp = true
validate_claims.iss = ["https://auth.example.com"]
```

## 生成测试 JWT

使用 [jwt.io](https://jwt.io/#debugger-io) 生成测试令牌用于开发。

## 依赖

- `rmqtt`（feature `plugin`）
- `jsonwebtoken`
- `reqwest`

## 许可证

MIT OR Apache-2.0
