require('dotenv').config();

const { Client, GatewayIntentBits, Events } = require('discord.js');
const { SimpleShardingStrategy } = require('@discordjs/ws');

const TOKEN = process.env.TOKEN;
const GATEWAY_PROXY_URL = process.env.GATEWAY_PROXY_URL; // e.g. ws://localhost:7878

if (!TOKEN) {
  console.error('TOKEN environment variable is required');
  process.exit(1);
}

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
  ],

  ws: GATEWAY_PROXY_URL
    ? {
        // Proxy manages identify rate limiting — disable client-side throttling.
        buildIdentifyThrottler: () => ({
          waitForIdentify: async () => {},
        }),

        // Override the gateway URL that shards connect to.
        //
        // discord.js fetches the URL from Discord's /gateway/bot endpoint and
        // caches it inside WebSocketManager. We patch fetchGatewayInformation
        // so that any URL Discord returns is replaced with the proxy URL.
        // This way shard count, session limits etc. still come from Discord,
        // but the WebSocket endpoint points at our proxy.
        buildStrategy: (manager) => {
          const originalFetch = manager.fetchGatewayInformation.bind(manager);

          manager.fetchGatewayInformation = async (force) => {
            const info = await originalFetch(force);
            return { ...info, url: GATEWAY_PROXY_URL };
          };

          return new SimpleShardingStrategy(manager);
        },
      }
    : {},
});

client.once(Events.ClientReady, (c) => {
  console.log(`Ready: logged in as ${c.user.tag}`);
  console.log(`Guilds: ${c.guilds.cache.size}`);
  console.log(`Gateway: ${GATEWAY_PROXY_URL ?? 'Discord (no proxy)'}`);
});

client.on(Events.MessageCreate, (message) => {
  if (message.author.bot) return;

  if (message.content === '!ping') {
    message.reply(`Pong! Latency: ${client.ws.ping}ms`);
  }
});

client.on(Events.Error, (error) => {
  console.error('WebSocket error:', error);
});

client.login(TOKEN);
