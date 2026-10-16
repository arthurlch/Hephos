# example-auth

Real token authentication: `argon2` password hashing, short-lived JWT access tokens
with a rotating refresh-token lifecycle, a framework auth seam
(`Router::authenticated`), and role-based authorization in handlers. No toy auth.

```
src/
  state.rs                 AppState (AuthService, UserService).
  domain/user.rs           User, Credentials, TokenPair, RefreshRequest, Claims.
  repos/users.rs           credential + profile queries.
  repos/refresh_tokens.rs  refresh-token storage (hashed), rotation, revocation.
  services/auth.rs         register, login, refresh (rotate), logout (revoke).
  services/users.rs        profile lookup.
  token.rs                 JWT issue/verify (keyring) + opaque refresh-token minting.
  api/auth.rs              public register/login/refresh/logout.
  api/users.rs             protected routes: GET /me, admin-only GET /users.
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
- Everything under the authenticated group requires the access token.
- `GET /me` works for any authenticated user; `GET /users` requires the `admin`
  role (`ctx.require_role("admin")`).

Only SHA-256 hashes of refresh tokens are stored, so a database dump cannot be
replayed as a session.

Requires Postgres and `JWT_SECRET`. Run:
`DATABASE_URL=… JWT_SECRET=… cargo run -p example-auth`

**Key rotation:** to rotate the signing key without downtime, set the new key as
`JWT_SECRET` and the old one as `JWT_SECRET_PREVIOUS`. New tokens are signed with the
current key; tokens signed with either key verify until you drop `JWT_SECRET_PREVIOUS`.
