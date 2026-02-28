// Dynamic client registry with optional database-backed polling.
//
// On startup, CLIENTS is seeded from config.json. If DATABASE_URL is set,
// a background task polls the database every second and atomically swaps
// the client map. This lets new guilds be added at runtime (e.g. when a
// user installs the bot) without restarting the proxy.

use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, RwLock},
    time::Duration,
};

use tracing::{error, info, warn};

use crate::config::{ClientConfig, CONFIG};

/// Dynamic client map. Initialized from config.json on startup, then
/// replaced by database contents if DATABASE_URL is set.
pub static CLIENTS: LazyLock<RwLock<HashMap<String, ClientConfig>>> =
    LazyLock::new(|| RwLock::new(CONFIG.clients.clone()));

/// Authenticate a WebSocket client by "client_id:secret" token.
/// Returns the set of authorized guild IDs if authentication succeeds.
pub fn authenticate_client(token: &str) -> Option<HashSet<u64>> {
    let (client_id, secret) = token.split_once(':')?;
    let clients = CLIENTS.read().ok()?;
    let client = clients.get(client_id)?;

    if client.secret == secret {
        Some(client.guilds.clone())
    } else {
        None
    }
}

const CREATE_TABLE_SQL: &str = "\
CREATE TABLE IF NOT EXISTS gateway_clients (
    client_id  TEXT NOT NULL,
    secret     TEXT NOT NULL,
    guild_id   TEXT NOT NULL,
    updated_at TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (client_id, guild_id)
)";

const SELECT_CLIENTS_SQL: &str =
    "SELECT client_id, secret, guild_id FROM gateway_clients";

/// Start polling the database for client config updates.
/// Reconnects automatically on connection failure.
pub async fn start_polling(database_url: String) {
    info!("Starting database config polling");

    loop {
        match run_poll_loop(&database_url).await {
            Ok(()) => break,
            Err(e) => {
                error!("Database polling failed: {e}, reconnecting in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn run_poll_loop(database_url: &str) -> Result<(), tokio_postgres::Error> {
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls).await?;

    // The connection object runs the actual I/O; must be spawned.
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            error!("Database connection lost: {e}");
        }
    });

    client.execute(CREATE_TABLE_SQL, &[]).await?;
    info!("Database connected, polling for client config every 1s");

    loop {
        let new_clients = poll_clients(&client).await?;

        if let Ok(mut clients) = CLIENTS.write() {
            *clients = new_clients;
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Query all client rows and group into the HashMap<client_id, ClientConfig>.
async fn poll_clients(
    client: &tokio_postgres::Client,
) -> Result<HashMap<String, ClientConfig>, tokio_postgres::Error> {
    let rows = client.query(SELECT_CLIENTS_SQL, &[]).await?;

    let mut clients: HashMap<String, ClientConfig> = HashMap::new();

    for row in rows {
        let client_id: String = row.get(0);
        let secret: String = row.get(1);
        let guild_id_str: String = row.get(2);

        let guild_id: u64 = match guild_id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                warn!("Invalid guild_id '{guild_id_str}' for client '{client_id}', skipping");
                continue;
            }
        };

        clients
            .entry(client_id.clone())
            .and_modify(|c| {
                if c.secret != secret {
                    warn!(
                        "Conflicting secrets for client '{client_id}', using first seen"
                    );
                }
                c.guilds.insert(guild_id);
            })
            .or_insert_with(|| {
                let mut guilds = HashSet::new();
                guilds.insert(guild_id);
                ClientConfig { secret, guilds }
            });
    }

    Ok(clients)
}
