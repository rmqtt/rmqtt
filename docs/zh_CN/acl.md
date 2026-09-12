[English](../en_US/acl.md)  | 简体中文

# 内置 ACL

内置 ACL 通过文件设置规则，使用上足够简单轻量，适用于规则数量可预测、无变动需求或变动较小的项目。

#### 插件：

```bash
rmqtt-acl
```

#### 插件配置文件：

```bash
plugins/rmqtt-acl.toml
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
内置 ACL 优先级最低，可以被 其它ACL 插件覆盖，如需禁用全部注释即可。规则文件更改后需重启 RMQTT服务 以应用生效。插件本身无法通过插件 API 停止（内置插件）；如需整体禁用，请将其加入 `plugins.disabled_default_startups`。
</font>
</div>


## 认证链中的 ACL 插件

`rmqtt-acl` 并不仅限于发布/订阅授权：它同时注册了 `ClientAuthenticate`，其规则会参与 CONNECT 阶段。它与认证插件共同构成一条按优先级排序的认证链——认证插件（如 `rmqtt-auth-http`，默认 priority=100）先执行，本插件（默认 priority=10）是链的末端成员：

- 命中 `allow` 规则 → 显式放行连接（认证链终止）；
- 命中 `deny` 规则 → 拒绝连接（`NotAuthorized`）；
- 无任何规则命中 → 同样拒绝连接（`NotAuthorized`）。

认证插件无法做出判定时将判定为 `ignore`，随后由认证链——也就是下面的规则——决定结果。注意默认末条规则 `["allow", "all"]` 省略了动作列，表示**包含 CONNECT 在内的所有操作**：被判定为 `ignore` 的连接会被该规则显式放行，即使 `allow_anonymous = false` 也是如此。

> **Fail-closed 加固：** 启用自定义认证插件时，请将 `["allow", "all"]` 注释掉并以 `["deny", "all"]` 作为末条规则，使只有被认证显式允许的客户端才能连接。认证侧的同一说明见 `rmqtt-auth-http` / `rmqtt-auth-jwt` 文档。

认证插件响应中携带的 ACL 规则（`rmqtt-auth-http` JSON 响应的 `acl` 字段、或 `rmqtt-auth-jwt` 令牌中的 `acl` 声明）会先由这些插件自身评估；仅当这些规则未产生判定时，才会应用本插件的文件规则。


## 定义 ACL

内置 ACL 是优先级最低规则表，在所有的 ACL 检查完成后，如果仍然未命中则检查默认的 ACL 规则。

该规则文件以 Toml 语法的格式进行描述：

```toml
rules = [
    # 允许 "dashboard" 用户 订阅 "$SYS/#" 主题
    ["allow", { user = "dashboard" }, "subscribe", ["$SYS/#"]],

    # 允许 IP 地址为 "127.0.0.1" 的客户端发布/订阅 "$SYS/#"，"#" 主题
    ["allow", { ipaddr = "127.0.0.1" }, "pubsub", ["$SYS/#", "#"]],
    
    # 拒绝 "所有用户" 订阅 "$SYS/#" "#" 主题
    ["deny", "all", "subscribe", ["$SYS/#", { eq = "#" }]],
    
    # 允许其它任意客户端连接以及发布/订阅操作
    #
    # 注意：["allow", "all"] 与 ["deny", "all"] 是互斥的末条规则——
    # 请根据部署方式保留其中一条（选择依据见下表）：
    #
    # * ["allow", "all"]——动作列省略，表示包含 CONNECT 在内的所有操作。
    #   适用于未启用自定义认证插件的独立部署：所有客户端均可连接，
    #   发布/订阅除被上方规则限制外默认允许。
    #
    # * ["deny", "all"]——启用自定义认证插件（rmqtt-auth-http、
    #   rmqtt-auth-jwt 等）时使用（fail-closed）：只有被认证显式允许的
    #   客户端才能连接；发布/订阅必须由认证插件返回的 ACL 数据（或上方
    #   的 allow 规则）授权。认证插件未做出判定的连接（'ignore'，例如
    #   认证服务返回 404/500）将被拒绝。
    ["allow", "all"]
    #["deny", "all"]
]
```

1. 第一条规则允许用户名为 `dashboard` 的客户端订阅 `$SYS/#` 主题，为第三条开了特例
2. 第二条规则允许 ip 地址为 `127.0.0.1` 的客户端发布/订阅 `$SYS/#` 与 `#` 主题，为第三条开了特例
3. 第三条规则禁止全部客户端订阅 `$SYS/#` 与 `#` 主题
4. 第四条规则允许全部客户端连接,发布/订阅所有主题

可知，默认的 ACL 主要是为了限制客户端对系统主题 `$SYS/#` 和全通配主题 `#` 的权限。

### `["allow", "all"]` 与 `["deny", "all"]` —— 如何选择

两条特殊的末条规则互斥：请根据部署方式保留其中一条。

| 末条规则 | 适用场景 | 效果 |
|----------|----------|------|
| `["allow", "all"]`（默认） | **未启用**自定义认证插件的独立部署 | 所有客户端均可连接；发布/订阅除被上方规则限制外默认允许 |
| `["deny", "all"]` | **启用**了自定义认证插件（`rmqtt-auth-http`、`rmqtt-auth-jwt` 等） | fail-closed：只有被认证显式允许的客户端才能连接；发布/订阅必须由认证插件返回的 ACL 数据（或上方的 allow 规则）授权。认证插件未做出判定的连接（`ignore`，例如认证服务返回 404/500）将被拒绝 |

注意 `["allow", "all"]` 同样覆盖 CONNECT（动作列省略表示所有操作），因此它可能把认证插件的 `ignore` 放大为连接成功——参见上文[“认证链中的 ACL 插件”](#认证链中的-acl-插件)。

## rmqtt-acl.toml 编写规则

`rmqtt-acl.toml` 文件中的规则按书写顺序从上往下匹配。

- 以 `#` 表示行注释。
- 每条规则由四元组组成。
- 元组第一位：表示规则命中成功后，执行权限控制操作，可取值为：
    * `allow`：表示 `允许`
    * `deny`： 表示 `拒绝`

- 元组第二位：表示规则所生效的用户，可使用的格式为：
    * `{ user = "dashboard" }`：表明规则仅对 *用户名 (Username)* 为 "dashboard" 的用户生效
    * `{ user = "dashboard", password = "123456", superuser = true }`：当元组第一位为allow时，可以设置password或superuser，表明规则对 *用户名 (
      Username)* 为 "dashboard" 且 *密码(Password)* 为 "123456" 的用户生效; superuser指示此用户为超级用户，在之后发布/订阅消息时将跳过认证直接允许操作。
    * `{ clientid = "dashboard" }`：表明规则仅对 *客户端标识 (ClientId)* 为 "dashboard" 的用户生效
    * `{ ipaddr = "127.0.0.1" }`：表明规则仅对 *源地址* 为 "127.0.0.1" 的用户生效
    * `{ protocol = 4 }`：表明规则仅对 *MQTT协议版本* 为 3.1.1 的用户生效. MQTT协议版本：3=3.1、4=3.1.1 或 5=5.0
    * `{ user = "dashboard", protocol = 4 }`：表明规则仅对 *用户名 (Username)* 为 "dashboard" 并且 *MQTT协议版本* 为 3.1.1  的用户生效
    * `all`：表明规则对所有的用户都生效

- 元组第三位：表示规则所控制的操作，可取值为：
    * `connect`：表明规则应用在 CONNECT 操作上
    * `publish`：表明规则应用在 PUBLISH 操作上
    * `subscribe`：表明规则应用在 SUBSCRIBE 操作上
    * `pubsub`：表明规则对 PUBLISH 和 SUBSCRIBE 操作都有效
    * `all`：表明规则对所有的操作都生效(默认)

- 元组第四位：表示规则所限制的主题列表，内容以数组的格式给出，例如：
    * `"$SYS/#"`：为一个 **主题过滤器 (Topic Filter)**；表示规则可命中与 `$SYS/#` 匹配的主题；如：可命中 "$SYS/#"，也可命中 "$SYS/a/b/c"
    * `{ eq = "#" }`：表示字符的全等，规则仅可命中主题为 `#` 的字串，不能命中 `/a/b/c` 等

- 除此之外还存在两条特殊的规则：
    - `{allow, all}`：允许所有操作
    - `{deny, all}`：拒绝所有操作

规则匹配细节：

- 规则按书写顺序自上而下评估。一条规则仅在**用户条件与主题条件同时命中**时才生效；若用户条件命中但主题条件未命中，则继续评估下一条规则。（作用于 CONNECT 的规则没有主题条件，仅由用户条件决定。）
- `password` 仅在 `allow` 规则中参与比对。`deny` 规则只按用户名匹配，会忽略已配置的 `password`。

在 `rmqtt-acl.toml` 修改完成后，并不会自动加载至 RMQTT 系统。需要手动执行：

```bash
curl -X PUT "http://127.0.0.1:6060/api/v1/plugins/1/rmqtt-acl/config/reload"
```

## 占位符

内置的 `rmqtt-acl.toml` 在主题的域（元组的第四位）仅支持以下占位符：

- `%c`： 表示客户端 ID，在规则生效时它将被替换为实际的客户端 ID。
- `%u`： 表示客户端的用户名，在规则生效时将被替换为实际的客户端用户名。

例如：

```
["allow", "all", "pubsub", ["sensor/%c/ctrl"]]
```

表示，**允许** 客户端 ID 为 `light` 的客户端 **订阅和发布** 到 `sensor/light/ctrl` 主题。

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
rmqtt-acl.toml 中应只包含一些简单而通用的规则，使其成为系统基础的 ACL 原则。如果需要支持复杂、大量的 ACL 内容，你应该在认证插件中去实现它。
</font>
</div>


