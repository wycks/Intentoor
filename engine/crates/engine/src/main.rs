use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    schema_version: String,
    #[serde(default)]
    emitted_at: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    normalized: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    raw: serde_json::Value,
}

#[derive(Debug, Clone)]
struct TrackedIntent {
    key: String,
    id: String,
    source: String,
    network: Option<String>,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    normalized: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct EngineState<'a> {
    schema_version: &'a str,
    emitted_at: DateTime<Utc>,
    active: Vec<EngineIntent<'a>>,
}

#[derive(Debug, Serialize)]
struct EngineIntent<'a> {
    key: &'a str,
    id: &'a str,
    source: &'a str,
    network: Option<&'a str>,
    ttl_seconds: i64,
    expires_at: DateTime<Utc>,
    sell_token: Option<&'a str>,
    buy_token: Option<&'a str>,
    sell_amount: Option<&'a str>,
    min_buy_amount: Option<&'a str>,

    // UniswapX Dutch auction (best-effort)
    sell_amount_start: Option<&'a str>,
    sell_amount_end: Option<&'a str>,
    buy_amount_start: Option<&'a str>,
    buy_amount_end: Option<&'a str>,
    decay_start_unix: Option<i64>,
    decay_end_unix: Option<i64>,
    exclusive_filler: Option<&'a str>,
    exclusive_until_unix: Option<i64>,
    order_type: Option<&'a str>,
}

fn main() -> Result<()> {
    let sub_endpoint =
        std::env::var("INGESTOR_SUB").unwrap_or_else(|_| "tcp://ingestor:5555".to_string());
    let pub_bind =
        std::env::var("ENGINE_PUB_BIND").unwrap_or_else(|_| "tcp://0.0.0.0:5556".to_string());
    let tick_ms: i64 = std::env::var("ENGINE_TICK_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    let default_ttl_seconds: i64 = std::env::var("DEFAULT_INTENT_TTL_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let ctx = zmq::Context::new();

    let sub = ctx.socket(zmq::SUB).context("create SUB")?;
    sub.set_subscribe(b"").context("subscribe")?;
    sub.connect(&sub_endpoint)
        .with_context(|| format!("connect SUB: {sub_endpoint}"))?;

    let pub_sock = ctx.socket(zmq::PUB).context("create PUB")?;
    pub_sock
        .bind(&pub_bind)
        .with_context(|| format!("bind PUB: {pub_bind}"))?;

    eprintln!(
        "engine: started sub={} pub={} tick_ms={} default_ttl_s={}",
        sub_endpoint, pub_bind, tick_ms, default_ttl_seconds
    );

    let mut book: HashMap<String, TrackedIntent> = HashMap::new();

    loop {
        // Poll for inbound messages with a timeout so we can tick/publish.
        let mut items = [sub.as_poll_item(zmq::POLLIN)];
        zmq::poll(&mut items, tick_ms).context("poll")?;

        if items[0].is_readable() {
            let msg = sub.recv_bytes(0).context("recv")?;
            if let Ok(env) = serde_json::from_slice::<Envelope>(&msg) {
                upsert_intent(&mut book, env, default_ttl_seconds);
            }
        }

        purge_expired(&mut book);
        publish_state(&pub_sock, &book).ok();
    }
}

fn upsert_intent(
    book: &mut HashMap<String, TrackedIntent>,
    env: Envelope,
    default_ttl_seconds: i64,
) {
    let now = Utc::now();

    let normalized = env.normalized.unwrap_or_default();

    let id = env.id.clone().unwrap_or_default();
    let raw_fingerprint = if id.is_empty() {
        // Stable-enough fallback when a venue doesn't provide an ID.
        // Hashing would be nicer, but we keep deps minimal for now.
        // This still dedupes within a single run reasonably.
        format!("raw:{}", env.raw.to_string().len())
    } else {
        id.clone()
    };

    let key = format!("{}:{}", env.source, raw_fingerprint);

    let expires_at = deadline_from_normalized(&normalized)
        .unwrap_or_else(|| now + chrono::Duration::seconds(default_ttl_seconds));

    match book.get_mut(&key) {
        Some(existing) => {
            existing.last_seen = now;
            existing.expires_at = expires_at;
            if !normalized.is_empty() {
                existing.normalized = normalized;
            }
        }
        None => {
            book.insert(
                key.clone(),
                TrackedIntent {
                    key,
                    id,
                    source: env.source,
                    network: env.network,
                    first_seen: now,
                    last_seen: now,
                    expires_at,
                    normalized,
                },
            );
        }
    }
}

fn deadline_from_normalized(n: &HashMap<String, serde_json::Value>) -> Option<DateTime<Utc>> {
    let deadline = n.get("deadline_unix")?.as_i64()?;
    DateTime::<Utc>::from_timestamp(deadline, 0)
}

fn purge_expired(book: &mut HashMap<String, TrackedIntent>) {
    let now = Utc::now();
    book.retain(|_, intent| intent.expires_at > now);
}

fn publish_state(pub_sock: &zmq::Socket, book: &HashMap<String, TrackedIntent>) -> Result<()> {
    let now = Utc::now();

    let mut active: Vec<&TrackedIntent> = book.values().collect();
    active.sort_by_key(|i| i.expires_at);

    let state = EngineState {
        schema_version: "engine-state/0.1.0",
        emitted_at: now,
        active: active
            .iter()
            .map(|i| {
                let ttl_seconds = (i.expires_at - now).num_seconds();
                EngineIntent {
                    key: &i.key,
                    id: &i.id,
                    source: &i.source,
                    network: i.network.as_deref(),
                    ttl_seconds,
                    expires_at: i.expires_at,
                    sell_token: i.normalized.get("sell_token").and_then(|v| v.as_str()),
                    buy_token: i.normalized.get("buy_token").and_then(|v| v.as_str()),
                    sell_amount: i.normalized.get("sell_amount").and_then(|v| v.as_str()),
                    min_buy_amount: i.normalized.get("min_buy_amount").and_then(|v| v.as_str()),

                    sell_amount_start: i
                        .normalized
                        .get("sell_amount_start")
                        .and_then(|v| v.as_str()),
                    sell_amount_end: i.normalized.get("sell_amount_end").and_then(|v| v.as_str()),
                    buy_amount_start: i
                        .normalized
                        .get("buy_amount_start")
                        .and_then(|v| v.as_str()),
                    buy_amount_end: i.normalized.get("buy_amount_end").and_then(|v| v.as_str()),
                    decay_start_unix: i
                        .normalized
                        .get("decay_start_unix")
                        .and_then(|v| v.as_i64()),
                    decay_end_unix: i.normalized.get("decay_end_unix").and_then(|v| v.as_i64()),
                    exclusive_filler: i
                        .normalized
                        .get("exclusive_filler")
                        .and_then(|v| v.as_str()),
                    exclusive_until_unix: i
                        .normalized
                        .get("exclusive_until_unix")
                        .and_then(|v| v.as_i64()),
                    order_type: i.normalized.get("order_type").and_then(|v| v.as_str()),
                }
            })
            .collect(),
    };

    let bytes = serde_json::to_vec(&state).context("serialize")?;
    pub_sock.send(bytes, 0).context("pub send")?;
    Ok(())
}
