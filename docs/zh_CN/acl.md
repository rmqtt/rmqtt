[English](../en_US/acl.md)  | 简体中文

# 内置 ACL

内置 ACL 通过文件设置规则，使用上足够简单轻量，适用于规则数量可预测、无变动需求或变动较小的项目。

ACL 规则文件：

```bash
plugins/rmqtt-acl.toml
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
内置 ACL 优先级最低，可以被 其它ACL 插件覆盖，如需禁用全部注释即可。规则文件更改后需重启 RMQTT服务 以应用生效。
</font>
</div>


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
    ["allow", "all"]
]
```

1. 第一条规则允许用户名为 `dashboard` 的客户端订阅 `$SYS/#` 主题，为第三条开了特例
2. 第二条规则允许 ip 地址为 `127.0.0.1` 的客户端发布/订阅 `$SYS/#` 与 `#` 主题，为第三条开了特例
3. 第三条规则禁止全部客户端订阅 `$SYS/#` 与 `#` 主题
4. 第四条规则允许全部客户端连接,发布/订阅所有主题

可知，默认的 ACL 主要是为了限制客户端对系统主题 `$SYS/#` 和全通配主题 `#` 的权限。

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

在 `rmqtt-acl.toml` 修改完成后，并不会自动加载至 RMQTT 系统。需要手动执行：

```bash
curl -X PUT "http://127.0.0.1:6066/api/v1/plugins/1/rmqtt-acl/config/reload"
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


