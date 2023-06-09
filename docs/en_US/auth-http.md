English | [简体中文](../zh_CN/auth-http.md)

# HTTP AUTH

HTTP authentication uses an external self-built HTTP application authentication data source, and determines the authentication result based on the data returned by the HTTP API, which can implement complex authentication logic.

Plugin：

```bash
rmqtt-auth-http
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
The rmqtt-auth-http plugin also includes ACL feature, which can be disabled via comments.
</font>
</div>

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
```json
HTTP/1.1 200 OK
X-Superuser: true
Content-Type: text/plain
Content-Length: 5
Date: Wed, 07 Jun 2023 01:29:23 GMT

allow
```

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

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
The POST and PUT methods are recommended. When using the GET method, the clear text password may be recorded with the URL in the server log during transmission.
</font>
</div>


# HTTP ACL

HTTP authentication utilizes an external self-built HTTP application as an authentication and authorization data source. It determines the authorization result based on the data returned by the HTTP API, enabling the implementation of complex ACL verification logic.

Plugin：

```bash
rmqtt-auth-http
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
The rmqtt-auth-http plugin includes authentication functionality and can be disabled by commenting out certain sections of the code.
</font>
</div>


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
- %r：Client Access Protocol
- %t：Topic

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
It is recommended to use POST and PUT methods. When using the GET method, the plain text password may be recorded in the server logs during the transmission process along with the URL.
</font>
</div>


# HTTP basic request information

HTTP API basic request information and headers.

```bash
# etc/plugins/rmqtt-auth-http.toml

## Request Header Configuration
http_timeout = "5s"
http_headers.accept = "*/*"
http_headers.Cache-Control = "no-cache"
http_headers.User-Agent = "RMQTT/0.2.11"
http_headers.Connection = "keep-alive"

# If publishing a message is rejected, the connection will be disconnected.
disconnect_if_pub_rejected = true

# If the HTTP request encounters an error, return "deny"; otherwise, return "ignore".
deny_if_error = true

```