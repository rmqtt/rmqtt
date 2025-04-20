English | [简体中文](../zh_CN/auth-jwt.md)


# JWT Auth

[JSON Web Token (JWT)](https://jwt.io/) is a token-based authentication mechanism that eliminates the need for the 
server to store client authentication credentials or session information. *RMQTT* supports user authentication based on JWT.

#### Authentication Principle

The client carries the JWT in the connection request, and the JWT signature is verified using a pre-configured secret 
key or public key. If the signature verification succeeds, the JWT authenticator proceeds to check the claims. The JWT 
authenticator actively verifies the validity of the JWT based on claims such as `nbf` (Not Before) and `exp` (Expiration Time). 
Additional custom claims can also be specified for authentication. The client is only granted access if both the signature 
and claims verification are successful.

#### Best Practice

Since the *RMQTT* JWT authenticator only verifies the JWT signature and cannot guarantee the legitimacy of the client's 
identity, it is recommended that users deploy a separate authentication server to issue JWTs for clients.

In this case, the client will first access the authentication server, where the server will verify the client's identity 
and issue a JWT for legitimate clients. The client will then use the issued JWT to connect to *RMQTT*.

#### Access Control List (Optional)

If the JWT contains an `acl` field, *RMQTT* will enforce access control for the client based on the permissions specified 
in that field. For more details, please refer to the [Access Control List (ACL)](./perm-list.md).

#### Plugins:

```bash
rmqtt-auth-jwt
```

#### Plugin configuration file:

```bash
plugins/rmqtt-auth-jwt.toml
```

#### Plugin configuration options:

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

Configure the following options:

 * JWT From(from): Specifies the location of the JWT in the client connection request. Optional values: `password` or 
   `username` (corresponding to the Password and Username fields in the MQTT client's CONNECT packet, respectively).
 * Algorithm(encrypt): Specifies the encryption method for the JWT. Optional values: `hmac-based` or `public-key`:
   * If `hmac-based` is selected, the JWT will use a symmetric key to generate and verify the signature (supporting 
     HS256, HS384, and HS512 algorithms). You should also configure:
     * Secret (hmac_secret): The key used to verify the signature, which is the same as the key used to generate the signature.
     * Secret Base64 Encode(hmac_base64): Configure whether *RMQTT* needs to perform Base64 decoding on the Secret before 
       verifying the signature. Optional values: `true` or `false`, with the default value set to `false`.
   * If `public-key` is selected, the JWT will use a private key to generate the signature, while a public key is needed 
     to verify the signature (supporting RS256, RS384, RS512, ES256, and ES384 algorithms). You should also configure:
     * Public Key：Specify the PEM-formatted public key used for verifying the signature.
 * Disconnect after expiration(disconnect_if_expiry)：Configure whether to disconnect the client connection after the JWT 
   expires. This feature is disabled by default.
 * Add custom claims checks(validate_claims): Users need to add keys and corresponding values in the Claim and Expected 
   Value fields, supporting the use of placeholders such as ${clientid}, ${username}, ${protocol}, and ${ipaddr}. The 
   key is used to look up the corresponding Claim in the JWT, while the value is compared against the actual value of the Claim.


By default, this plugin is not enabled. To activate it, you must add the `rmqtt-auth-jwt` entry to the
`plugins.default_startups` configuration in the main configuration file `rmqtt.toml`, as shown below:
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

