English | [简体中文](../zh_CN/auth-http.md)

# HTTP AUTH

HTTP authentication uses an external self-built HTTP application authentication data source, and determines the authentication result based on the data returned by the HTTP API, which can implement complex authentication logic.

#### Plugins:

```bash
rmqtt-auth-http
```

#### Plugin configuration file:

```bash
plugins/rmqtt-auth-http.toml
```

#### Plugin configuration options:

```bash
##--------------------------------------------------------------------
## rmqtt-auth-http
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/auth-http.md

http_timeout = "5s"
http_headers.accept = "*/*"
http_headers.Cache-Control = "no-cache"
http_headers.User-Agent = "RMQTT/0.8.0"
http_headers.Connection = "keep-alive"

## Disconnect if publishing is rejected
##
## Value: true | false
## Default: true
disconnect_if_pub_rejected = true

## Disconnect After Expiration
##
## Value: true | false
## Default: false
disconnect_if_expiry = false

##Return 'Deny' if http request error otherwise 'Ignore'
##
## Value: true | false
## Default: true
deny_if_error = true

##--------------------------------------------------------------------
## Authentication request.
##
## Variables:
##  - %u: username
##  - %c: clientid
##  - %a: ipaddress
##  - %r: protocol
##  - %P: password
##
## Value: URL
http_auth_req.url = "http://127.0.0.1:9090/mqtt/auth"
## Value: post | get | put
http_auth_req.method = "post"
## HTTP request header of authentication request
## Content-Type Currently supported values: application/x-www-form-urlencoded, application/json
http_auth_req.headers = { content-type = "application/x-www-form-urlencoded" }
#http_auth_req.headers.content-type="application/json"
## Value: Params
http_auth_req.params = { clientid = "%c", username = "%u", password = "%P" }


##--------------------------------------------------------------------
## ACL request.
##
## Variables:
##  - %A: 1 | 2, 1 = sub, 2 = pub
##  - %u: username
##  - %c: clientid
##  - %a: ipaddress
##  - %r: protocol
##  - %t: topic
##
## Value: URL
http_acl_req.url = "http://127.0.0.1:9090/mqtt/acl"
## Value: post | get | put
http_acl_req.method = "post"
## Value: Params
http_acl_req.params = { access = "%A", username = "%u", clientid = "%c", ipaddr = "%a", topic = "%t" }
```

> **TIP**    
> The rmqtt-auth-http plugin also includes ACL feature, which can be disabled via comments.


## Authentication principle

RMQTT Broker: In the device connection event, use the current client-related information as parameters to send a request to the user's custom authentication service for querying permissions. The authentication request is processed based on the returned HTTP response information.

- Successful authentication:
  - API returns a 2xx status code with the message body: allow.
- Failed authentication:
  - API returns a 2xx status code with the message body: deny.
  - API HTTP request fails and deny_if_error configuration is set to true.
- Ignore authentication:
  - API returns a 2xx status code with the message body: ignore, and the authentication chain continues.
  - API returns a 4xx/5xx status code, and the message body is ignored, resulting in the authentication chain continuing with a result of ignore.
  - API HTTP request fails and deny_if_error configuration is set to false, resulting in the authentication chain continuing with a result of ignore.
- Superuser:
  - Successful authentication with the response header "X-Superuser: true". Superusers bypass ACL authorization.


Response examples:
```
HTTP/1.1 200 OK
X-Superuser: true
Content-Type: text/plain
Content-Length: 5
Date: Wed, 07 Jun 2023 01:29:23 GMT

allow
```

Starting from *RMQTT* version 0.8.0, you can set an optional `acl` field in the response body to specify client permissions. For more details, please refer to the [Access Control List (ACL)](./perm-list.md).

Starting from *RMQTT* version 0.8.0, you can set an optional `expire_at` field in the response body to specify the client's authentication expiration time and force the client to disconnect for reauthentication. The value should be a Unix timestamp (in seconds).


Response examples:
```json
HTTP/1.1 200 OK
Content-Length: 565
Content-Type: application/json; charset=utf-8
Date: Wed, 25 Sep 2024 01:54:37 GMT

{
  "result": "allow",  // "allow" | "deny" | "ignore"
  "superuser": false,  // true | false, If this field is empty, the default value is `false`
  "expire_at": 1827143027,  // Optional, authentication expiration time
  "acl": [
    {
      "action": "all",
      "permission": "allow",
      "topic": "foo/${clientid}"
    },
    {
      "action": "subscribe",
      "permission": "allow",
      "qos": [1, 2],
      "topic": "eq foo/1/#"
    },
    {
      "action": "publish",
      "permission": "deny",
      "retain": true,
      "topic": "foo/4"
    }
  ]
}
```

> **TIP**    
> To support predefined ACL permissions, the *Content-Type* must be set to 'application/json' format.


## Authentication request

When performing authentication, RMQTT will use the current client information to populate and initiate a user-configured authentication query request. This request is used to retrieve the authentication data of the client from the HTTP server.

```bash
# etc/plugins/rmqtt-auth-http.toml

## Request address
http_auth_req.url = "http://127.0.0.1:9090/mqtt/auth"

## HTTP request method
## Value: post | get | put
http_auth_req.method = "post"

## HTTP Request Headers for Auth Request, Content-Type header is configured by default.
## The possible values of the Content-Type header: application/x-www-form-urlencoded, application/json
http_auth_req.headers = { content-type = "application/x-www-form-urlencoded" }

## Request parameter
http_auth_req.params = { clientid = "%c", username = "%u", password = "%P" }
```

When the HTTP request method is GET, the request parameters will be passed in the form of a URL query string; Under POST and PUT requests, it will submit the request parameters in the form of Json or ordinary form (determined by the value of content-type).

You can use the following placeholders in the authentication request, and RMQTT Broker will be automatically populated with client information when requested:

- %u：Username
- %c：Client ID
- %a：Client IP address
- %r：Client Access Protocol
- %P：Clear text password


> **TIP<br>**
> The POST and PUT methods are recommended. When using the GET method, the clear text password may be recorded with the URL in the server log during transmission.


# HTTP ACL

HTTP authentication utilizes an external self-built HTTP application as an authentication and authorization data source. It determines the authorization result based on the data returned by the HTTP API, enabling the implementation of complex ACL verification logic.

Plugin：

```bash
rmqtt-auth-http
```

> **TIP<br>**
> The rmqtt-auth-http plugin includes authentication functionality and can be disabled by commenting out certain sections of the code.

To enable HTTP ACL, the following needs to be configured in `etc/plugins/rmqtt-auth-http.toml`:

## ACL Authentication principle

RMQTT uses the current client-related information as parameters in device publish and subscribe events to send a request to the user's custom authentication service for permission. The ACL authorization request is processed based on the returned HTTP response information.

- Successful authorization:
  - API returns a 2xx status code with the message body: allow.
- No permissions:
  - API returns a 2xx status code with the message body: deny.
  - API HTTP request fails and deny_if_error configuration is set to true.
- Ignore authorization:
  - API returns a 2xx status code with the message body: ignore, and the authorization chain continues.
  - API returns a 4xx/5xx status code, and the message body is ignored, resulting in the authorization chain continuing with a result of ignore.
  - API HTTP request fails and deny_if_error configuration is set to false, resulting in the authorization chain continuing with a result of ignore.
- Cache authorization result:
  - Response header returns "X-Cache: -1" indicating the result is cached. The value represents the cache timeout duration in milliseconds. The value -1 indicates it is valid during the active connection period.
  - **Note: Caching is not supported for subscription ACLs.**

When performing publish and subscribe authentication, RMQTT will populate the current client information and initiate a user-configured ACL authorization query request. This request is used to retrieve the authorization data of the client from the HTTP server.

If the user is a superuser, the ACL authorization authentication will be skipped.

## ACL authorization query request

```bash
# etc/plugins/rmqtt-auth-http.toml

## Request address
http_acl_req.url = "http://127.0.0.1:9090/mqtt/acl"

## HTTP request method
## Value: post | get | put
http_acl_req.method = "get"

## HTTP Request Headers for Auth Request, Content-Type header is configured by default.
## The possible values of the Content-Type header: application/x-www-form-urlencoded, application/json
#http_acl_req.headers = { content-type = "application/x-www-form-urlencoded" }

## Request parameter
http_acl_req.params = { access = "%A", username = "%u", clientid = "%c", ipaddr = "%a", topic = "%t" }

```

## Request description

When the HTTP request method is GET, the request parameters will be passed in the form of URL query strings. For POST and PUT requests, the parameters will be submitted as a regular form in the format of "x-www-form-urlencoded" (content-type: x-www-form-urlencoded).

You can use the following placeholders in the authentication request, and RMQTT will automatically populate them with client information:

%u:User name
%c:Client ID
%a:Client IP address
%r:Client Access Protocol
%P:Clear text password
%p:Client Port

- %A：Operation types: '1' for subscribe, '2' for publish.
- %u：User name
- %c：Client ID
- %a：Client IP address
- %r：The MQTT protocol version accessed by the client. The values are: 3=3.1, 4=3.1.1 or 5=5.0
- %t：Topic

> **TIP<br>**
> It is recommended to use POST and PUT methods. When using the GET method, the plain text password may be recorded in 
> the server logs during the transmission process along with the URL.

# HTTP basic request information

HTTP API basic request information and headers.

```bash
# etc/plugins/rmqtt-auth-http.toml

## Request Header Configuration
http_timeout = "5s"
http_headers.accept = "*/*"
http_headers.Cache-Control = "no-cache"
http_headers.User-Agent = "RMQTT/0.8.0"
http_headers.Connection = "keep-alive"

# If publishing a message is rejected, the connection will be disconnected.
disconnect_if_pub_rejected = true

# If the HTTP request encounters an error, return "deny"; otherwise, return "ignore".
deny_if_error = true

```