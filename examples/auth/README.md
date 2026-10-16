# example-auth

Real token authentication: `argon2id` password hashing (explicit OWASP-2024 cost) with
a centralized password policy, short-lived JWT access tokens with a rotating
refresh-token lifecycle, machine-to-machine API keys, an async framework auth seam
(`Router::authenticated`), and role-based authorization in handlers. No toy auth.

```
src/
  state.rs                 AppState (AuthService, UserService).
  domain/user.rs           User, Credentials, TokenPair, RefreshRequest,
                           CreateApiKey, ApiKeyCreated, Claims.
  repos/users.rs           credential + profile queries.
  repos/refresh_tokens.rs  refresh-token storage (hashed), rotation, revocation.
  repos/api_keys.rs        API-key storage (hashed), lookup, revocation.
  services/auth.rs         register, login, refresh, logout, authenticate (the seam
                           resolver: JWT or API key), mint/revoke API keys.
  services/users.rs        profile lookup.
  token.rs                 JWT issue/verify (keyring) + opaque refresh-token and
                           API-key minting (hashed).
  api/auth.rs              public register/login/refresh/logout.
  api/users.rs             protected routes: GET /me, admin-only GET /users.
  api/keys.rs              admin-only POST /api-keys, DELETE /api-keys/{id}.
```

Flow:
- `POST /auth/register` and `POST /auth/login` are public. Login returns
  `{ access_token, refresh_token }`: the access token is a short-lived (15 min) JWT
  sent as `Authorization: Bearer <jwt>`; the refresh token is a long-lived (30 day)
  opaque string.
- `POST /auth/refresh` with `{ "refresh_token": … }` rotates the token — it revokes
  the presented one and returns a fresh pair. Presenting an already-rotated token is
  treated as theft: the user's entire token family is revoked (`401`).
- `POST /auth/logout` with `{ "refresh_token": … }` revokes it (`204`, and opaque —
  an unknown token also returns `204`, so logout is not an existence oracle).
- Everything under the authenticated group requires a credential — either the access
  token or a machine API key — in the `Authorization: Bearer` header.
- `GET /me` works for any authenticated user; `GET /users` requires the `admin`
  role (`ctx.require_role("admin")`).

**Machine-to-machine API keys.** An admin mints a key with `POST /api-keys`
(`{ "name": …, "roles": [...] }`); the raw `hephos_sk_…` value is returned once and
never again. A caller presents it exactly like a JWT (`Authorization: Bearer
hephos_sk_…`) — the seam recognizes the prefix, looks the key up, and resolves it to
the same `Identity` (id = key id, roles = the key's scopes). `DELETE /api-keys/{id}`
revokes it. The seam (`AuthService::authenticate`) is async precisely because this
lookup is a database query.

Only SHA-256 hashes of refresh tokens and API keys are stored, so a database dump
cannot be replayed as a session or a key.

Requires Postgres and `JWT_SECRET`. Run:
`DATABASE_URL=… JWT_SECRET=… cargo run -p example-auth`

**Key rotation:** to rotate the signing key without downtime, set the new key as
`JWT_SECRET` and the old one as `JWT_SECRET_PREVIOUS`. New tokens are signed with the
current key; tokens signed with either key verify until you drop `JWT_SECRET_PREVIOUS`.
