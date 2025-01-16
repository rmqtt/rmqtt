[English](../en_US/auth-jwt.md)  | 简体中文

# JWT 认证

[JSON Web Token (JWT)](https://jwt.io/) 是一种基于 Token 的认证机制。它不需要服务器来保留客户端的认证信息或会话信息。*RMQTT* 支持基于 
JWT 进行用户认证。

#### 认证原理

客户端在连接请求中携带 JWT，将使用预先配置的密钥或公钥对 JWT 签名进行验证。如果签名验证成功，JWT 认证器将继续检查声明。JWT 认证器会根据这些声
明如 nbf（不早于）和 exp（过期时间）来主动检查 JWT 的有效性。还可以指定额外的自定义声明进行认证。只有当签名和声明的认证都成功时，客户端才被授权访问。

#### 推荐用法

由于 *RMQTT* JWT 认证器只会检查 JWT 的签名，无法对客户端身份的合法性提供担保，因此推荐用户部署一个独立的认证服务器用来为客户端颁发 JWT。

此时，客户端将首先访问该认证服务器，由该认证服务器验证客户端的身份，并为合法的客户端签发 JWT，之后客户端将使用签发的 JWT 来连接 *RMQTT*。

#### 权限列表

如果 JWT 中包含 acl 字段，*RMQTT* 将根据该字段指定的权限对客户端进行访问控制。 详情请参考 [权限列表（ACL）](./perm-list.md)。

#### 插件：

```bash
rmqtt-auth-jwt
```

#### 插件配置文件：

```bash
plugins/rmqtt-auth-jwt.toml
```

#### 插件配置项：

```bash

##--------------------------------------------------------------------
## rmqtt-auth-jwt
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/auth-jwt.md

#Disconnect if publishing is rejected
disconnect_if_pub_rejected = true

## From where the JWT string can be got
## Value: username | password
## Default: password
from = "password"

## Encryption method
## Value: hmac-based | public-key
## Default: hmac-based
encrypt = "hmac-based"

## HMAC Hash Secret.
##
## Value: String
hmac_secret = "rmqttsecret"
#hmac_secret = "cm1xdHRzZWNyZXQ="

## Secret Base64 Encode
##
## Value: true | false
## Default: false
hmac_base64 = false

## RSA or ECDSA public key file.
##
## Value: File
public_key = "./rmqtt-bin/jwt_public_key_rsa.pem"

## Disconnect After Expiration
##
## Value: true | false
## Default: false
disconnect_if_expiry = false

## The checklist of claims to validate
##
## Value: String
## validate_claims.$name = expected
##
## Placeholder:
##  - ${username}: username
##  - ${clientid}: clientid
##  - ${ipaddr}: client ip addr
##  - ${protocol}: MQTT protocol version: 3 = 3.1, 4 = 3.1.1, or 5 = 5.0

### Basic Validation
## > Validate the token's expiration by comparing the exp claim to the current UTC time.
validate_claims.exp = true
## < Ensure the token is not used before its nbf claim.
#validate_claims.nbf = true
## Ensure the token's subject (sub claim) is as expected.
#validate_claims.sub = "user@rmqtt.com"
## Validate the token's issuer by comparing the iss claim to the known issuer.
#validate_claims.iss = ["https://rmqtt.com1", "https://rmqtt.com"]
## Verify that the token's audience (aud claim) matches the intended recipient.
#validate_claims.aud = ["https://your-api.com", "mobile_app", "web_app"]

### Extended Validation
#validate_claims.clientid = "${clientid}"
#validate_claims.username = "${username}"
#validate_claims.ipaddr = "${ipaddr}"
#validate_claims.protocol = "${protocol}"

```

配置说明：
 * JWT 来自于(from)：指定客户端连接请求中 JWT 的位置；可选值： password、 username（分别对应于 MQTT 客户端 CONNECT 报文中的 Password 
   和 Username 字段）。
 * 加密方式(encrypt)：指定 JWT 的加密方式，可选值： hmac-based、public-key：
   * 如选择 hmac-based，即 JWT 使用对称密钥生成签名和校验签名（支持 HS256、HS384 和 HS512 算法），还应配置：
     * Secret(hmac_secret)：用于校验签名的密钥，与生成签名时使用的密钥相同。
     * Secret Base64 Encode(hmac_base64)：配置 *RMQTT* 在使用 Secret 校验签名时是否需要先对其进行 Base64 解密；可选值：true、false，默认值：false。
   * 如选择 public-key，即 JWT 使用私钥生成签名，同时需要使用公钥校验签名（支持 RS256、RS384、RS512、ES256、ES384 算法），还应配置：
     * Public Key：指定用于校验签名的 PEM 格式的公钥。
 * 过期后断开连接(disconnect_if_expiry)：配置是否在 JWT 过期后断开客户端连接，默认未启用。
 * 添加自定义的 Claims 检查(validate_claims): 用户需要在 Claim 和 Expected Value 分别添加键和对应的值，支持使用 ${clientid}、${username}、
   ${protocol}、${ipaddr} 占位符。其中键用于查找 JWT 中对应的 Claim，值则用于与 Claim 的实际值进行比较。


默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-auth-jwt”项，如：
```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "rmqtt-plugins/"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-retainer",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-sys-topic",
    #"rmqtt-message-storage",
    #"rmqtt-session-storage",
    "rmqtt-auth-jwt",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

