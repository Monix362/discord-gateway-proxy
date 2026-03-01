#!/usr/bin/env tsx
/**
 * Fly.io deployment for the gateway-proxy (Discord gateway WebSocket proxy).
 * Cross-compiles Rust binary from macOS to Linux x86_64 musl, then deploys
 * a minimal scratch Docker image to fly.io.
 *
 * Config is hardcoded here except for TOKEN which comes from Doppler
 * (project: 'website', stage: 'production').
 *
 * Usage:
 *   pnpm run deploy
 *   SKIP_BUILD=1 pnpm run deploy   # skip cross-compilation, use existing binary
 */
import {
    deployFly,
    getDopplerEnv,
    shell,
} from '@xmorse/deployment-utils'

const appName = 'kimaki-gateway-production'

const gatewayConfig = {
    log_level: 'info',
    intents: 32511,
    externally_accessible_url: `wss://${appName}.fly.dev`,
    cache: {
        channels: false,
        presences: false,
        emojis: false,
        current_member: false,
        members: false,
        roles: false,
        scheduled_events: false,
        stage_instances: false,
        stickers: false,
        users: false,
        voice_states: false,
    },
}

async function main() {
    const stage = 'production'

    const env = await getDopplerEnv({ stage, project: 'website' })

    if (!env.DISCORD_BOT_TOKEN) {
        throw new Error('DISCORD_BOT_TOKEN not found in Doppler')
    }

    const config = {
        ...gatewayConfig,
        token: env.DISCORD_BOT_TOKEN,
    }

    if (!process.env.SKIP_BUILD) {
        await shell(`pnpm run build:linux`, { cwd: process.cwd() })
    }

    await deployFly({
        appName,
        port: 7878,
        dockerfile: 'Dockerfile.fly',
        buildRemotely: true,
        forceHttps: false,
        machineType: 'shared-cpu-1x',
        memorySize: '512mb',
        strategy: 'immediate',
        minInstances: 1,
        maxInstances: 1,
        regions: ['iad'],
        env: {
            ...env,
            CONFIG: JSON.stringify(config),
        },
    })
}

main()
