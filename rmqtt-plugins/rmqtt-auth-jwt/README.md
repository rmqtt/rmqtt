[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-auth-jwt

[![crates.io](https://img.shields.io/crates/v/rmqtt-auth-jwt.svg)](https://crates.io/crates/rmqtt-auth-jwt)

JWT authentication plugin for RMQTT. Validates JSON Web Tokens for client authentication.

## Overview

Validates JWT tokens extracted from the client's password or username field. Supports HMAC-based (HS256/HS384/HS512) and public-key (RS256/RS384/RS512, ES256/ES384/ES512) encryption. Provides claim validation including `exp`, `nbf`, `sub`, `iss`, `aud`, and extended custom claims.

### Authentication Flow

1. Client connects and provides a JWT in the password or username field
2. Plugin extracts the JWT from the configured field (`from`)
3. Based on `encrypt` setting:
   - **hmac-based**: Decodes using the HMAC secret (`hmac_secret`), optionally Base64-decoded (`hmac_base64`)
   - **public-key**: Decodes using the RSA/ECDSA public key file (`public_key`)
4. Validates standard claims (`exp`, `nbf`, `sub`, `iss`, `aud`) if configured
5. Validates extended custom claims against actual client attributes
6. If validation passes → authentication success; otherwise → authentication failure

### Supported Algorithms

| Category | Algorithms |
|----------|------------|
| HMAC | HS256, HS384, HS512 |
| RSA | RS256, RS384, RS512 |
| ECDSA | ES256, ES384, ES512 |

## Usage

Add the dependency to `Cargo.toml`:

```toml
rmqtt-auth-jwt = "0.22"
```

Register the plugin in your broker startup code:

```rust
rmqtt_auth_jwt::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-auth-jwt.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `disconnect_if_pub_rejected` | Boolean | `true` | Disconnect client if publish is rejected |
| `disconnect_if_expiry` | Boolean | `false` | Disconnect client after expiry |
| `from` | String | `"password"` | Source of JWT: `"username"` or `"password"` |
| `encrypt` | String | `"hmac-based"` | Encryption method: `"hmac-based"` or `"public-key"` |
| `hmac_secret` | String | `"rmqttsecret"` | HMAC hash secret |
| `hmac_base64` | Boolean | `false` | Whether the HMAC secret is Base64-encoded |
| `public_key` | String (File) | *(none)* | RSA or ECDSA public key file path (required when `encrypt = "public-key"`) |
| `validate_claims.exp` | Boolean | `true` | Validate token expiration (`exp` claim) |
| `validate_claims.nbf` | Boolean | `false` | Validate not-before (`nbf` claim) |
| `validate_claims.sub` | String | *(none)* | Expected subject (`sub` claim) value |
| `validate_claims.iss` | String[] | *(none)* | Expected issuer(s) (`iss` claim) |
| `validate_claims.aud` | String[] | *(none)* | Expected audience(s) (`aud` claim) |
| `validate_claims.clientid` | String | *(none)* | Expected client ID (supports `${clientid}` placeholder) |
| `validate_claims.username` | String | *(none)* | Expected username (supports `${username}` placeholder) |
| `validate_claims.ipaddr` | String | *(none)* | Expected IP address (supports `${ipaddr}` placeholder) |
| `validate_claims.protocol` | String | *(none)* | Expected protocol version (supports `${protocol}` placeholder) |

### Claim Validation Placeholders

When configuring extended claims (`clientid`, `username`, `ipaddr`, `protocol`), the following placeholders are available to match dynamic client attributes:

| Placeholder | Description |
|-------------|-------------|
| `${username}` | Client username |
| `${clientid}` | Client ID |
| `${ipaddr}` | Client IP address |
| `${protocol}` | MQTT protocol version: `3` = 3.1, `4` = 3.1.1, `5` = 5.0 |

When a claim is set to a placeholder like `${clientid}`, it means the JWT's `clientid` claim must match the actual client's client ID.

## Example Configuration

### HMAC-based (default)

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

### Public-key (RSA/ECDSA)

```toml
from = "password"
encrypt = "public-key"
public_key = "./rmqtt-bin/jwt_public_key_rsa.pem"

validate_claims.exp = true
validate_claims.iss = ["https://auth.example.com"]
```

## Generate Test JWT

Use [jwt.io](https://jwt.io/#debugger-io) to generate test tokens for development.

## Dependencies

- `rmqtt` (feature `plugin`)
- `jsonwebtoken`
- `reqwest`

## License

MIT OR Apache-2.0
