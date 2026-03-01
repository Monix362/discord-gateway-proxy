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
