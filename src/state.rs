use rand::{distributions::Alphanumeric, thread_rng, Rng};
use tokio::sync::{broadcast, Notify};
use twilight_gateway::MessageSender;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crate::{cache, dispatch::BroadcastMessage, model::JsonObject};

const SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// Manager for the READY state of a shard.
pub struct Ready {
    inner: RwLock<Option<JsonObject>>,
    changed: Notify,
}

impl Ready {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
            changed: Notify::new(),
        }
    }

    pub async fn wait_changed(&self) {
        self.changed.notified().await;
    }

    pub fn is_ready(&self) -> bool {
        self.inner.read().unwrap().is_some()
    }

    pub fn set_ready(&self, payload: JsonObject) {
        *self.inner.write().unwrap() = Some(payload);
        self.changed.notify_waiters();
    }

    pub fn set_not_ready(&self) {
        *self.inner.write().unwrap() = None;
        self.changed.notify_waiters();
    }

    pub async fn wait_until_ready(&self) -> JsonObject {
        while !self.is_ready() {
            self.wait_changed().await;
        }

        self.inner.read().unwrap().clone().unwrap()
    }
}

/// State of a single shard.
pub struct Shard {
    /// ID of this shard.
    pub id: u32,
    /// Sender for this shard.
    pub sender: MessageSender,
    /// Handle for broadcasting events for this shard.
    pub events: broadcast::Sender<BroadcastMessage>,
    /// READY state manager for this shard.
    pub ready: Ready,
    /// Cache for guilds on this shard.
    pub guilds: cache::Guilds,
}

/// A session initiated by a client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionPrincipal {
    BotToken,
    Client(String),
    Unvalidated(String),
}

/// A session initiated by a client.
#[derive(Clone)]
pub struct Session {
    /// Shard ID that this session is for.
    pub shard_id: u32,
    /// Compression as requested in IDENTIFY.
    pub compress: Option<bool>,
    /// Auth principal used to create this session.
    pub principal: SessionPrincipal,
    /// Guild IDs this client is authorized to receive events for.
    /// None means all events are forwarded (legacy behavior).
    /// Some(set) means only events for guilds in the set are forwarded.
    pub authorized_guilds: Option<Arc<HashSet<u64>>>,
    /// Last time this session was used by a client.
    pub last_accessed: Instant,
}

/// Global state for all shards managed by the proxy.
pub struct Inner {
    /// State of all shards managed by the proxy.
    pub shards: Vec<Arc<Shard>>,
    /// Total shard count.
    pub shard_count: u32,
    /// All sessions active in the proxy.
    pub sessions: RwLock<HashMap<String, Session>>,
}

impl Inner {
    fn prune_expired_sessions(sessions: &mut HashMap<String, Session>) {
        let now = Instant::now();
        sessions.retain(|_, session| now.duration_since(session.last_accessed) <= SESSION_TTL);
    }

    /// Get a session by its ID.
    pub fn get_session(&self, session_id: &str) -> Option<Session> {
        let mut sessions = self.sessions.write().unwrap();
        Self::prune_expired_sessions(&mut sessions);

        let session = sessions.get_mut(session_id)?;
        session.last_accessed = Instant::now();

        Some(session.clone())
    }

    /// Create a new session.
    pub fn create_session(&self, session: Session) -> String {
        let mut sessions = self.sessions.write().unwrap();
        Self::prune_expired_sessions(&mut sessions);

        let mut rng = thread_rng();
        loop {
            // Session IDs are 32 bytes of ASCII.
            let session_id: String = std::iter::repeat(())
                .map(|()| rng.sample(Alphanumeric))
                .map(char::from)
                .take(32)
                .collect();

            if sessions.contains_key(&session_id) {
                continue;
            }

            sessions.insert(session_id.clone(), session);
            return session_id;
        }
    }
}

/// A reference to the [`StateInner`] of the proxy.
pub type State = Arc<Inner>;
