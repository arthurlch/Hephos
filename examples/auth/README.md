# example-auth

Real token authentication: `argon2` password hashing, stateless JWT access
tokens, a framework auth seam (`Router::authenticated`), and role-based
authorization in handlers. No toy auth.

```
src/
  state.rs           AppState (Db, AuthService, UserService).
  domain/user.rs     User, Credentials, TokenResponse, Claims.
  repos/users.rs     credential + profile queries.
  services/auth.rs   register (hash), login (verify + issue token).
  services/users.rs  profile lookup.
  api/auth.rs        public login/register; verify_token() for the auth seam.
  api/users.rs       protected routes: GET /me, admin-only GET /users.
```

Flow:
- `POST /auth/register` and `POST /auth/login` are public.
- Everything under the authenticated group requires `Authorization: Bearer <jwt>`.
- `GET /me` works for any authenticated user; `GET /users` requires the `admin`
  role (`ctx.require_role("admin")`).

Requires Postgres and `JWT_SECRET`. Run:
`DATABASE_URL=… JWT_SECRET=… cargo run -p example-auth`
