#[cfg(feature = "simd-json")]
use halfbrown::hashmap;
use serde::Serialize;
#[cfg(not(feature = "simd-json"))]
use serde_json::Value as OwnedValue;
#[cfg(feature = "simd-json")]
use simd_json::OwnedValue;
use tracing::warn;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use crate::model::JsonObject;

/// Wrapper used to serialize READY payloads. op is a raw u8 (= 0 for Dispatch).
#[derive(Serialize)]
pub struct Payload<T: Serialize> {
    pub d: T,
    pub op: u8,
    pub t: &'static str,
    pub s: usize,
}

struct GuildEntry {
    /// Pre-serialized JSON of the `d` object from Discord's GUILD_CREATE.
    /// Stored as a raw string so clients receive it without any re-parsing.
    json: String,
    unavailable: bool,
}

struct GuildStateInner {
    /// guild_id → entry with full GUILD_CREATE `d` JSON
    guilds: HashMap<u64, GuildEntry>,
    /// channel_id → guild_id, used by resolve_guild_id_for_channel
    channel_index: HashMap<u64, u64>,
}

pub struct GuildCacheStats {
    pub guilds: usize,
    pub channels: usize,
}

/// Per-shard guild cache backed by raw JSON storage instead of per-resource
/// typed DashMap entries. One entry per guild stores the pre-serialized
/// GUILD_CREATE `d` JSON sent to clients directly — no reconstruction step.
///
/// Memory: O(guilds) HashMap entries instead of O(total_resources) DashMap
/// entries. Typically 5-20x lower memory than twilight InMemoryCache for
/// large bots.
pub struct Guilds(Arc<RwLock<GuildStateInner>>);

impl Guilds {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(GuildStateInner {
            guilds: HashMap::new(),
            channel_index: HashMap::new(),
        })))
    }

    pub fn stats(&self) -> GuildCacheStats {
        let state = self.0.read().unwrap();
        GuildCacheStats {
            guilds: state.guilds.len(),
            channels: state.channel_index.len(),
        }
    }

    pub fn resolve_guild_id_for_channel(&self, channel_id: u64) -> Option<u64> {
        self.0.read().unwrap().channel_index.get(&channel_id).copied()
    }

    /// Process a dispatch event and update the guild cache.
    /// Only events that change guild state are handled; all others are no-ops.
    pub fn process_event(&self, event_name: &str, payload: &str, guild_id: Option<u64>) {
        match event_name {
            "GUILD_CREATE" => self.on_guild_create(payload),
            "GUILD_UPDATE" => self.on_guild_update(payload, guild_id),
            "GUILD_DELETE" => self.on_guild_delete(payload, guild_id),
            "CHANNEL_CREATE" => self.on_channel_add(payload, guild_id, false),
            "THREAD_CREATE" => self.on_channel_add(payload, guild_id, true),
            "CHANNEL_UPDATE" | "THREAD_UPDATE" => self.on_channel_or_thread_update(payload, guild_id),
            "CHANNEL_DELETE" | "THREAD_DELETE" => self.on_channel_delete(payload, guild_id),
            "THREAD_LIST_SYNC" => self.on_thread_list_sync(payload, guild_id),
            "ROLE_CREATE" | "ROLE_UPDATE" => self.on_role_upsert(payload, guild_id),
            "ROLE_DELETE" => self.on_role_delete(payload, guild_id),
            "GUILD_EMOJIS_UPDATE" => self.on_field_replace(payload, guild_id, "emojis"),
            "GUILD_STICKERS_UPDATE" => self.on_field_replace(payload, guild_id, "stickers"),
            "VOICE_STATE_UPDATE" => self.on_voice_state_update(payload, guild_id),
            _ => {}
        }
    }

    pub fn get_ready_payload(
        &self,
        mut ready: JsonObject,
        sequence: &mut usize,
        authorized_guilds: Option<&HashSet<u64>>,
    ) -> Payload<JsonObject> {
        *sequence += 1;

        let state = self.0.read().unwrap();

        let guilds: Vec<_> = state
            .guilds
            .keys()
            .filter(|guild_id| authorized_guilds.map_or(true, |g| g.contains(guild_id)))
            .map(|guild_id| {
                #[cfg(feature = "simd-json")]
                {
                    OwnedValue::Object(Box::new(hashmap! {
                        String::from("id") => guild_id.to_string().into(),
                        String::from("unavailable") => true.into(),
                    }))
                }
                #[cfg(not(feature = "simd-json"))]
                {
                    serde_json::json!({
                        "id": guild_id.to_string(),
                        "unavailable": true
                    })
                }
            })
            .collect();

        ready.insert(String::from("guilds"), OwnedValue::Array(guilds.into()));

        Payload {
            d: ready,
            op: 0,
            t: "READY",
            s: *sequence,
        }
    }

    /// Iterate GUILD_CREATE / GUILD_DELETE payloads for a connecting client.
    /// The pre-stored guild JSON is wrapped with the op/t/s envelope directly
    /// via string concatenation — no re-parsing or re-serialization needed.
    pub fn get_guild_payloads<'a>(
        &'a self,
        sequence: &'a mut usize,
        authorized_guilds: Option<&'a HashSet<u64>>,
    ) -> impl Iterator<Item = String> + 'a {
        let state = self.0.read().unwrap();
        let entries: Vec<(u64, String, bool)> = state
            .guilds
            .iter()
            .filter(|(guild_id, _)| authorized_guilds.map_or(true, |g| g.contains(guild_id)))
            .map(|(guild_id, entry)| (*guild_id, entry.json.clone(), entry.unavailable))
            .collect();
        // Drop read lock before returning the iterator.
        drop(state);

        entries.into_iter().map(move |(guild_id, json, unavailable)| {
            *sequence += 1;
            let s = *sequence;
            if unavailable {
                format!(
                    r#"{{"op":0,"t":"GUILD_DELETE","s":{s},"d":{{"id":"{guild_id}","unavailable":true}}}}"#
                )
            } else {
                format!(r#"{{"op":0,"t":"GUILD_CREATE","s":{s},"d":{json}}}"#)
            }
        })
    }

    // --- Event handlers ---

    fn on_guild_create(&self, payload: &str) {
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to parse GUILD_CREATE payload: {e}");
                return;
            }
        };
        let d = &value["d"];

        let guild_id = match d["id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            Some(id) => id,
            None => {
                warn!("GUILD_CREATE missing guild id");
                return;
            }
        };

        let unavailable = d["unavailable"].as_bool().unwrap_or(false);

        let guild_json = match serde_json::to_string(d) {
            Ok(j) => j,
            Err(e) => {
                warn!("Failed to serialize guild {guild_id}: {e}");
                return;
            }
        };

        let mut state = self.0.write().unwrap();

        // Index all channels and threads for resolve_guild_id_for_channel
        for array_key in &["channels", "threads"] {
            if let Some(items) = d[array_key].as_array() {
                for item in items {
                    if let Some(ch_id) =
                        item["id"].as_str().and_then(|s| s.parse::<u64>().ok())
                    {
                        state.channel_index.insert(ch_id, guild_id);
                    }
                }
            }
        }

        state
            .guilds
            .insert(guild_id, GuildEntry { json: guild_json, unavailable });
    }

    fn on_guild_update(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let d = value["d"].clone();

        // GUILD_UPDATE only contains top-level guild scalar fields — no resource
        // arrays (channels, roles, members) — so merging all fields is safe.
        self.modify_guild(guild_id, |guild| {
            if let (Some(update), Some(stored)) = (d.as_object(), guild.as_object_mut()) {
                for (key, val) in update {
                    stored.insert(key.clone(), val.clone());
                }
            }
        });
    }

    fn on_guild_delete(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value =
            serde_json::from_str(payload).unwrap_or(serde_json::Value::Null);
        let unavailable = value["d"]["unavailable"].as_bool().unwrap_or(false);

        let mut state = self.0.write().unwrap();
        if unavailable {
            if let Some(entry) = state.guilds.get_mut(&guild_id) {
                entry.unavailable = true;
            }
        } else {
            state.guilds.remove(&guild_id);
            state.channel_index.retain(|_, gid| *gid != guild_id);
        }
    }

    fn on_channel_add(&self, payload: &str, guild_id: Option<u64>, is_thread: bool) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let d = value["d"].clone();

        let channel_id = match d["id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            Some(id) => id,
            None => return,
        };
        let channel_id_str = d["id"].as_str().unwrap_or("").to_string();
        let array_key = if is_thread { "threads" } else { "channels" };

        let mut state = self.0.write().unwrap();
        state.channel_index.insert(channel_id, guild_id);
        Self::modify_guild_inner(&mut state, guild_id, |guild| {
            if let Some(arr) = guild.get_mut(array_key).and_then(|v| v.as_array_mut()) {
                arr.retain(|ch| ch["id"].as_str() != Some(&channel_id_str));
                arr.push(d.clone());
            }
        });
    }

    fn on_channel_or_thread_update(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let d = value["d"].clone();
        let channel_id_str = match d["id"].as_str() {
            Some(s) => s.to_string(),
            None => return,
        };

        self.modify_guild(guild_id, |guild| {
            for array_key in &["channels", "threads"] {
                if let Some(arr) = guild.get_mut(*array_key).and_then(|v| v.as_array_mut()) {
                    if let Some(ch) =
                        arr.iter_mut().find(|ch| ch["id"].as_str() == Some(&channel_id_str))
                    {
                        *ch = d.clone();
                        return;
                    }
                }
            }
        });
    }

    fn on_channel_delete(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let d = &value["d"];
        let channel_id = match d["id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
            Some(id) => id,
            None => return,
        };
        let channel_id_str = channel_id.to_string();

        let mut state = self.0.write().unwrap();
        state.channel_index.remove(&channel_id);
        Self::modify_guild_inner(&mut state, guild_id, |guild| {
            for array_key in &["channels", "threads"] {
                if let Some(arr) = guild.get_mut(*array_key).and_then(|v| v.as_array_mut()) {
                    arr.retain(|ch| ch["id"].as_str() != Some(&channel_id_str));
                }
            }
        });
    }

    fn on_thread_list_sync(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let d = &value["d"];
        let threads = match d["threads"].as_array() {
            Some(t) => t.clone(),
            None => return,
        };

        let mut state = self.0.write().unwrap();
        for thread in &threads {
            if let Some(ch_id) = thread["id"].as_str().and_then(|s| s.parse::<u64>().ok()) {
                state.channel_index.insert(ch_id, guild_id);
            }
        }
        let threads_value = serde_json::Value::Array(threads);
        Self::modify_guild_inner(&mut state, guild_id, |guild| {
            guild["threads"] = threads_value.clone();
        });
    }

    fn on_role_upsert(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        // ROLE_CREATE / ROLE_UPDATE: d = { guild_id, role }
        let role = value["d"]["role"].clone();
        let role_id_str = match role["id"].as_str() {
            Some(s) => s.to_string(),
            None => return,
        };

        self.modify_guild(guild_id, |guild| {
            if let Some(arr) = guild.get_mut("roles").and_then(|v| v.as_array_mut()) {
                if let Some(existing) =
                    arr.iter_mut().find(|r| r["id"].as_str() == Some(&role_id_str))
                {
                    *existing = role.clone();
                } else {
                    arr.push(role.clone());
                }
            }
        });
    }

    fn on_role_delete(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        // ROLE_DELETE: d = { guild_id, role_id }
        let role_id_str = match value["d"]["role_id"].as_str() {
            Some(s) => s.to_string(),
            None => return,
        };

        self.modify_guild(guild_id, |guild| {
            if let Some(arr) = guild.get_mut("roles").and_then(|v| v.as_array_mut()) {
                arr.retain(|r| r["id"].as_str() != Some(&role_id_str));
            }
        });
    }

    fn on_voice_state_update(&self, payload: &str, guild_id: Option<u64>) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let d = value["d"].clone();
        let user_id_str = match d["user_id"].as_str() {
            Some(s) => s.to_string(),
            None => return,
        };
        let left_channel = d["channel_id"].is_null();

        self.modify_guild(guild_id, |guild| {
            if let Some(arr) = guild.get_mut("voice_states").and_then(|v| v.as_array_mut()) {
                arr.retain(|vs| vs["user_id"].as_str() != Some(&user_id_str));
                if !left_channel {
                    arr.push(d.clone());
                }
            } else if !left_channel {
                guild["voice_states"] = serde_json::Value::Array(vec![d.clone()]);
            }
        });
    }

    /// Generic handler for events that wholesale replace a top-level array field
    /// (GUILD_EMOJIS_UPDATE → "emojis", GUILD_STICKERS_UPDATE → "stickers").
    fn on_field_replace(&self, payload: &str, guild_id: Option<u64>, field: &'static str) {
        let Some(guild_id) = guild_id else { return };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(_) => return,
        };
        let new_array = value["d"][field].clone();
        if new_array.is_null() {
            return;
        }

        self.modify_guild(guild_id, |guild| {
            guild[field] = new_array.clone();
        });
    }

    // --- Helpers ---

    /// Parse the stored guild JSON, apply a mutation via `f`, and reserialize.
    fn modify_guild<F: FnOnce(&mut serde_json::Value)>(&self, guild_id: u64, f: F) {
        let mut state = self.0.write().unwrap();
        Self::modify_guild_inner(&mut state, guild_id, f);
    }

    fn modify_guild_inner<F: FnOnce(&mut serde_json::Value)>(
        state: &mut GuildStateInner,
        guild_id: u64,
        f: F,
    ) {
        if let Some(entry) = state.guilds.get_mut(&guild_id) {
            match serde_json::from_str::<serde_json::Value>(&entry.json) {
                Ok(mut value) => {
                    f(&mut value);
                    match serde_json::to_string(&value) {
                        Ok(new_json) => entry.json = new_json,
                        Err(e) => warn!("Failed to reserialize guild {guild_id}: {e}"),
                    }
                }
                Err(e) => warn!("Failed to parse stored guild {guild_id}: {e}"),
            }
        }
    }
}
