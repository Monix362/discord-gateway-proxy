// Shared authentication for gateway WebSocket and REST proxy paths.

use std::{collections::HashSet, sync::Arc};

use crate::{config::CONFIG, db_config, state::SessionPrincipal};

pub struct AuthContext {
    pub principal: SessionPrincipal,
    pub authorized_guilds: Option<Arc<HashSet<u64>>>,
}

pub fn normalize_gateway_token(token: &str) -> &str {
    token.split_whitespace().last().unwrap_or("")
}

pub fn authenticate_gateway_token(token: &str) -> Option<AuthContext> {
    if let Some((client_id, guilds)) = db_config::authenticate_client_with_id(token) {
        return Some(AuthContext {
            principal: SessionPrincipal::Client(client_id),
            authorized_guilds: Some(Arc::new(guilds)),
        });
    }

    if token == CONFIG.token {
        return Some(AuthContext {
            principal: SessionPrincipal::BotToken,
            authorized_guilds: None,
        });
    }

    if CONFIG.validate_token {
        return None;
    }

    Some(AuthContext {
        principal: SessionPrincipal::Unvalidated(token.to_string()),
        authorized_guilds: None,
    })
}
