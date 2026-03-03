<!-- Agent guidance for gateway-proxy contributors and automation. -->

# gateway-proxy scope

gateway-proxy is a Discord proxy for both **Gateway WebSocket** and
**Discord REST** traffic.

- Handle websocket upgrade, IDENTIFY/RESUME auth, shard fanout, event filtering,
  and gateway session behavior.
- Handle REST `/api/v10/*` forwarding with the same client auth model used for
  guild-scoped WS filtering.
- Exposed operational endpoints: `/metrics`, `/shard-count`.

# split with website

Onboarding flows are handled by the `website` package.

- gateway-proxy owns REST proxying (`/api/v10/*`) and `client_id:secret`
  authorization for REST requests.
- website handles OAuth callback and onboarding status endpoints only.

When changing one side of this split, document and validate compatibility with
the other side.

# deploying

ALWAYS use the deploy script to deploy gateway-proxy. NEVER use `fly deploy` directly.

```bash
cd gateway-proxy && pnpm run deploy
```

This cross-compiles the Rust binary locally on macOS (via `build:linux` using `x86_64-linux-musl-gcc`), then deploys a minimal scratch image with `Dockerfile.fly`. The `Dockerfile` (non-fly) is only for reference and is NOT used for deployment.

To skip the build step (e.g. re-deploy with same binary):

```bash
SKIP_BUILD=1 pnpm run deploy
```

The deploy script reads secrets from Doppler (project: `website`, stage: `production`) and sets them as Fly secrets automatically.

# database TLS

The gateway connects to PlanetScale Postgres which requires TLS. The code uses
`tokio-postgres-rustls` with Mozilla root CAs (`webpki-roots`). `tokio-postgres`
only supports `sslmode` values `disable`, `prefer`, `require` — it rejects
`verify-full` and `verify-ca` as "invalid connection string". The
`normalize_database_url()` function in `db_config.rs` rewrites those to
`sslmode=require` before connecting.
