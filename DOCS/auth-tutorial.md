# pacsnode Authentication Tutorial

This guide covers the two supported authentication modes in pacsnode:

- local multi-user authentication managed by pacsnode itself
- external OIDC bearer-token validation for deployments that already have an identity provider

## 1. Choose a mode

Use local auth when you want pacsnode to manage users, password policy, refresh tokens, and the first bootstrap admin account.

Use OIDC mode when login happens elsewhere and pacsnode only needs to validate incoming bearer tokens from a provider such as Keycloak or Auth0.

## 2. Local Authentication Tutorial

### 2.1 Enable the plugins

In `config.toml`:

```toml
[plugins]
enabled = ["basic-auth", "admin-dashboard"]

[plugins.basic-auth]
mode = "local"
jwt_secret = "replace-with-a-long-random-secret"
access_token_ttl_secs = 900
refresh_token_ttl_secs = 604800
public_paths = ["/health", "/metrics"]
```

Notes:

- `audit-logger` is auto-enabled by default for secured deployments unless you explicitly opt out.
- `admin-dashboard` is optional, but it is the easiest way to manage users and password policy after the first login.

### 2.2 Start pacsnode

```bash
./target/release/pacsnode
```

### 2.3 Bootstrap the first admin user

Run this once after enabling `basic-auth`:

```bash
./target/release/pacsnode create-admin --username admin --email admin@example.test
```

The command prints a one-time generated password that already satisfies the stored password policy.

### 2.4 Log in and obtain tokens

```bash
curl -s http://localhost:8042/auth/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "<generated-password>"
  }'
```

Example response:

```json
{
  "access_token": "<jwt>",
  "refresh_token": "<refresh-token>",
  "token_type": "Bearer",
  "expires_in": 900
}
```

### 2.5 Call protected endpoints

```bash
export PACS_TOKEN="<jwt>"

curl -s http://localhost:8042/auth/me \
  -H "Authorization: Bearer $PACS_TOKEN"

curl -s http://localhost:8042/api/studies \
  -H "Authorization: Bearer $PACS_TOKEN"

curl -s http://localhost:8042/wado/studies?limit=10 \
  -H "Authorization: Bearer $PACS_TOKEN"
```

### 2.6 Refresh or revoke tokens

Refresh an access token:

```bash
curl -s http://localhost:8042/auth/refresh \
  -H "Content-Type: application/json" \
  -d '{"refresh_token": "<refresh-token>"}'
```

Revoke refresh tokens for the current user:

```bash
curl -i -X POST http://localhost:8042/auth/logout \
  -H "Authorization: Bearer $PACS_TOKEN"
```

### 2.7 Manage users and password policy

After the first admin login, use the admin dashboard for day-to-day administration:

- users
- password policy
- server settings
- registered DICOM nodes
- audit review

Default route:

```text
/admin
```

The built-in role set is:

- `admin`
- `radiologist`
- `technologist`
- `viewer`
- `uploader`

## 3. OIDC Bearer Validation Tutorial

In OIDC mode, pacsnode does not perform the browser login flow. It validates bearer tokens that were issued elsewhere.

### 3.1 Issuer-discovery mode

This is the simplest setup for standards-compliant providers.

```toml
[plugins]
enabled = ["basic-auth"]

[plugins.basic-auth]
mode = "oidc"
public_paths = ["/health", "/metrics"]

[plugins.basic-auth.oidc]
issuer = "https://keycloak.example.test/realms/pacs"
audience = "pacsnode"
role_claim = "realm_access.roles"
attributes_claims = ["department", "modality_access", "email"]
role_map = { pacs_admin = "admin", pacs_viewer = "viewer" }
```

When neither `jwks_uri` nor `public_key_pem` is set, pacsnode resolves:

```text
/.well-known/openid-configuration
```

from the configured issuer and discovers the provider JWKS endpoint automatically.

### 3.2 Explicit JWKS mode

Use this when you want to pin the JWKS URL directly:

```toml
[plugins.basic-auth.oidc]
issuer = "https://keycloak.example.test/realms/pacs"
audience = "pacsnode"
jwks_uri = "https://keycloak.example.test/realms/pacs/protocol/openid-connect/certs"
jwks_refresh_secs = 300
role_claim = "realm_access.roles"
role_map = { pacs_admin = "admin" }
```

### 3.3 Static RSA key mode

Use this for offline or tightly controlled environments where the signing key is fixed:

```toml
[plugins.basic-auth.oidc]
issuer = "https://issuer.example.test/realms/pacs"
audience = "pacsnode"
public_key_pem = "${PACS_OIDC_PUBLIC_KEY_PEM}"
role_claim = "realm_access.roles"
role_map = { pacs_admin = "admin" }
```

### 3.4 Forward bearer tokens to pacsnode

After your IdP login completes, pass the resulting access token through unchanged:

```bash
export PACS_TOKEN="<external-access-token>"

curl -s http://localhost:8042/auth/me \
  -H "Authorization: Bearer $PACS_TOKEN"

curl -s http://localhost:8042/wado/studies?limit=10 \
  -H "Authorization: Bearer $PACS_TOKEN"
```

In OIDC mode, `/auth/login`, `/auth/refresh`, and `/auth/logout` are not used.

## 4. Claim Mapping

Default OIDC claim handling:

| Setting | Default |
|---------|---------|
| `user_id_claim` | `sub` |
| `username_claim` | `preferred_username` |
| `role_claim` | `roles` |

Behavior:

- `user_id_claim` must resolve to a string or number
- `username_claim` falls back to the user ID if it is absent
- `role_claim` may be a string or an array of strings
- `attributes_claims` are copied into the authenticated user attribute bag

Nested claims are supported with dotted paths, for example:

```toml
role_claim = "realm_access.roles"
attributes_claims = ["department", "claims.site", "email"]
```

## 5. Operational Notes

- Local mode uses pacsnode-managed refresh tokens; OIDC mode validates access tokens only.
- JWKS mode caches signing keys and refreshes on expiry or unknown `kid`.
- Discovery mode caches the discovered JWKS location and then uses the same cached JWKS validation path.
- Account lockout is enforced only for local users because OIDC logins happen outside pacsnode.
- Native HTTP and DIMSE TLS are not implemented yet; terminate TLS at a reverse proxy.

## 6. Troubleshooting

### `401 Unauthorized`

Check:

- the `Authorization: Bearer <token>` header is present
- the token issuer matches `issuer`
- the audience matches `audience`
- the token contains the expected `kid` when using discovery or JWKS mode

### `invalid oidc claims`

Check:

- `user_id_claim`, `username_claim`, and `role_claim`
- your `role_map`
- whether nested claim paths are correct

### Admin pages return `403 Forbidden`

The current user must resolve to the `admin` role.

## 7. Related Docs

- [README](../README.md)
- [Feature matrix](feature-matrix.md)
- [OHIF server requirements](ohif-server-requirements.md)