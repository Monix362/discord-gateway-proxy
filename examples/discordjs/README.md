# discord.js Example

This example shows how to connect a discord.js v14 bot to gateway-proxy instead of Discord's gateway directly.

## How it works

- REST requests are routed through the proxy at `http://localhost:7878/api`, so discord.js fetches `/gateway/bot` from the proxy and receives the proxy's WebSocket URL (`externally_accessible_url` in config).
- discord.js then connects its shards to the proxy instead of Discord.
- Client-side identify throttling is disabled since the proxy handles all rate limiting.

## Setup

### 1. Configure gateway-proxy

Make sure `externally_accessible_url` is set in `config.json`:

```json
{
  "token": "YOUR_BOT_TOKEN",
  "intents": 33281,
  "port": 7878,
  "externally_accessible_url": "ws://localhost:7878"
}
```

### 2. Install dependencies

```bash
npm install
```

### 3. Configure environment

```bash
cp .env.example .env
# Edit .env and set TOKEN to your bot token
```

The `TOKEN` must match what is configured in gateway-proxy.

### 4. Start gateway-proxy

```bash
# From the project root
cargo run --release
```

### 5. Start the bot

```bash
npm start
```

## Multi-tenant mode

If the proxy uses `clients` config, set `TOKEN` to `client_id:secret` in `.env`:

```env
TOKEN=us-east:random-secret-us-east
```

discord.js will use this as the authorization value in IDENTIFY, and the proxy will route only the authorized guilds' events to this client.
