// crates/pocketflow-core/src/store.rs
//
// SharedStore — dual-backend (in-memory for dev, Redis for production).
// Same interface regardless of backend. Swap via REDIS_URL env var.

use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::RwLock;
use tracing::debug;
use fred::prelude::*;

// ── Event ring buffer ─────────────────────────────────────────────────────

const RING_BUFFER_SIZE: usize = 1000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoreEvent {
    pub agent: String,
    pub event_type: String,
    pub payload: Value,
    pub ts: u64, // unix millis
}

// ── Backend trait (sealed inside this module) ─────────────────────────────

#[async_trait::async_trait]
trait StoreBackend: Send + Sync {
    async fn get(&self, key: &str) -> Option<Value>;
    async fn set(&self, key: &str, value: Value);
    async fn del(&self, key: &str);
}

// ── In-memory backend ─────────────────────────────────────────────────────

struct InMemoryBackend {
    map: RwLock<HashMap<String, Value>>,
}

impl InMemoryBackend {
    fn new() -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl StoreBackend for InMemoryBackend {
    async fn get(&self, key: &str) -> Option<Value> {
        self.map.read().await.get(key).cloned()
    }
    async fn set(&self, key: &str, value: Value) {
        self.map.write().await.insert(key.to_string(), value);
    }
    async fn del(&self, key: &str) {
        self.map.write().await.remove(key);
    }
}

// ── Redis backend ─────────────────────────────────────────────────────────
// Allow redis as stub for now
// Allow redis as stub for now
struct RedisBackend {
    client: std::sync::Arc<fred::clients::Client>,
}

impl RedisBackend {
    async fn new(url: &str) -> Result<Self> {
        use fred::prelude::*;
        let config = Config::from_url(url)?;
        let client = Builder::from_config(config).build()?;
        client.init().await?;
        Ok(Self { client: Arc::new(client) })
    }
}

#[async_trait::async_trait]
impl StoreBackend for RedisBackend {
    async fn get(&self, key: &str) -> Option<Value> {
        use fred::prelude::*;
        let raw: Option<String> = self.client.get(key).await.ok()?;
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }
    async fn set(&self, key: &str, value: Value) {
        use fred::prelude::*;
        if let Ok(s) = serde_json::to_string(&value) {
            let _: core::result::Result<(), _> =
                self.client.set::<(), _, _>(key, s, None, None, false).await;
        }
    }
    async fn del(&self, key: &str) {
        use fred::prelude::*;
        let _: core::result::Result<i64, _> = self.client.del(key).await;
    }
}

// ── SharedStore (public API) ──────────────────────────────────────────────

#[derive(Clone)]
pub struct SharedStore {
    backend: Arc<dyn StoreBackend>,
    ring_buffer: Arc<RwLock<Vec<StoreEvent>>>,
    /// Monotonically increasing count of events that have been evicted from
    /// the front of the ring buffer. Adding this to a buffer index gives the
    /// absolute sequence number for that event.
    base_seq: Arc<AtomicUsize>,
    // Optional Redis client used as a cross-process event bus when present.
    redis_client: Option<Arc<fred::clients::Client>>,
}

impl SharedStore {
    /// In-memory backend — use for dev and tests.
    pub fn new_in_memory() -> Self {
        Self {
            backend: Arc::new(InMemoryBackend::new()),
            ring_buffer: Arc::new(RwLock::new(Vec::with_capacity(RING_BUFFER_SIZE))),
            base_seq: Arc::new(AtomicUsize::new(0)),
            redis_client: None,
        }
    }

    /// Redis backend — use for Docker Compose and production.
    pub async fn new_redis(url: &str) -> Result<Self> {
        // Initialise Redis backend + populate local ring buffer from the Redis list
        let backend_impl = RedisBackend::new(url).await?;
        let client = Arc::clone(&backend_impl.client);

        // Load recent events from Redis into the in-memory ring buffer so
        // cross-process events become visible to this process.
        let mut init_buf: Vec<StoreEvent> = Vec::with_capacity(RING_BUFFER_SIZE);
        if let Ok(len) = client.llen::<i64, _>("pocketflow:events").await {
            if len > 0 {
                if let Ok(items) = client.lrange::<Vec<String>, _>("pocketflow:events", 0, -1).await {
                    for item in items {
                        if let Ok(ev) = serde_json::from_str::<StoreEvent>(&item) {
                            init_buf.push(ev);
                        }
                    }
                }
            }
        }

        Ok(Self {
            backend: Arc::new(backend_impl),
            ring_buffer: Arc::new(RwLock::new(init_buf)),
            base_seq: Arc::new(AtomicUsize::new(0)),
            redis_client: Some(client),
        })
    }

    // ── Core get/set/del ─────────────────────────────────────────────

    pub async fn get(&self, key: &str) -> Option<Value> {
        let v = self.backend.get(key).await;
        debug!(key, found = v.is_some(), "store.get");
        v
    }

    pub async fn set(&self, key: &str, value: Value) {
        debug!(key, "store.set");
        self.backend.set(key, value).await;
    }

    pub async fn del(&self, key: &str) {
        debug!(key, "store.del");
        self.backend.del(key).await;
    }

    /// Typed get — deserialises JSON into T. Returns None on missing key or type mismatch.
    pub async fn get_typed<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let v = self.get(key).await?;
        serde_json::from_value(v).ok()
    }

    /// Typed set — serialises T to JSON Value.
    pub async fn set_typed<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let v = serde_json::to_value(value)?;
        self.set(key, v).await;
        Ok(())
    }

    // ── Event ring buffer ─────────────────────────────────────────────

    /// Emit a structured event. Every node lifecycle phase should call this.
    pub async fn emit(&self, agent: &str, event_type: &str, payload: Value) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = StoreEvent {
            agent: agent.to_string(),
            event_type: event_type.to_string(),
            payload,
            ts,
        };

        // Push into in-memory ring buffer immediately for local consumers.
        let mut buf = self.ring_buffer.write().await;
        if buf.len() >= RING_BUFFER_SIZE {
            buf.remove(0);
            self.base_seq.fetch_add(1, Ordering::Relaxed);
        }
        buf.push(event.clone());
        drop(buf);

        // If Redis available, also RPUSH the serialized event into a
        // shared Redis list so other processes can observe it.
        if let Some(client) = &self.redis_client {
                if let Ok(s) = serde_json::to_string(&event) {
                let _: core::result::Result<i64, _> = client.rpush("pocketflow:events", s).await;
                // Trim to keep the list bounded to the ring buffer size.
                let _: Result<(), _> = client.ltrim("pocketflow:events", -(RING_BUFFER_SIZE as i64), -1).await;
            }
        }
    }

    /// Returns `(new_cursor, events)` where `cursor` is an absolute sequence
    /// number.  `new_cursor` is always the absolute index one past the last
    /// returned event and should replace the caller's stored cursor.
    ///
    /// If the ring buffer has evicted events that `cursor` once pointed to
    /// (i.e. `cursor < base_seq`), the method clamps to the oldest available
    /// event so no *future* events are silently skipped.
    pub async fn get_events_since(&self, cursor: usize) -> (usize, Vec<StoreEvent>) {
        // If Redis is configured, try to pull any new events from the
        // shared Redis list into the local ring buffer first. This lets
        // processes that only write to Redis be observed here.
        if let Some(client) = &self.redis_client {
            if let Ok(remote_len_i64) = client.llen::<i64, _>("pocketflow:events").await {
                let remote_len = remote_len_i64 as usize;
                let local_len = { self.ring_buffer.read().await.len() };
                if remote_len > local_len {
                    // Fetch items from the Redis list starting at local_len.
                    if let Ok(items) = client.lrange::<Vec<String>, _>("pocketflow:events", local_len as i64, -1).await {
                        if !items.is_empty() {
                            let mut buf = self.ring_buffer.write().await;
                            for item in items {
                                if let Ok(ev) = serde_json::from_str::<StoreEvent>(&item) {
                                    if buf.len() >= RING_BUFFER_SIZE {
                                        buf.remove(0);
                                        self.base_seq.fetch_add(1, Ordering::Relaxed);
                                    }
                                    buf.push(ev);
                                }
                            }
                        }
                    }
                }
            }
        }

        let buf = self.ring_buffer.read().await;
        let base = self.base_seq.load(Ordering::Relaxed);
        // `cursor` is an absolute sequence number; convert to a buffer index,
        // clamping downward if the ring wrapped past the cursor position.
        let buf_idx = cursor.saturating_sub(base).min(buf.len());
        let events = buf[buf_idx..].to_vec();
        let new_cursor = base + buf.len();
        (new_cursor, events)
    }

    /// Number of events in the ring buffer (for initial TUI render).
    pub async fn event_count(&self) -> usize {
        self.ring_buffer.read().await.len()
    }
}
