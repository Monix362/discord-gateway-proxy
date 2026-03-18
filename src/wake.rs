// Wake helpers for internet-reachable kimaki clients.
// Sends POST /kimaki/wake to the client's reachable URL and waits until
// kimaki reports discord.js is connected.

use std::{sync::LazyLock, time::Duration};

use tracing::warn;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(35))
        .build()
        .expect("wake client")
});

pub async fn wake_client(client_id: &str, reachable_url: &str, token: &str) {
    let endpoint = format!("{}/kimaki/wake", reachable_url.trim_end_matches('/'));

    let response = HTTP_CLIENT
        .post(endpoint.clone())
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await;

    match response {
        Ok(res) => {
            if !res.status().is_success() {
                warn!(
                    "Wake request failed for client '{client_id}': endpoint={endpoint}, status={}",
                    res.status()
                );
            }
        }
        Err(error) => {
            warn!(
                "Wake request error for client '{client_id}': endpoint={endpoint}, error={error}"
            );
        }
    }
}
