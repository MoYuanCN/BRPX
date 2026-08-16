use super::storage::Database;
use async_channel::{Receiver, Sender};
use chrono::Utc;
use rand::distr::{Alphanumeric, SampleString};
use rusqlite::{params, params_from_iter, types::Value};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::sync::Semaphore;

pub const BUSINESS_CODE_HEADER: &str = "x-brpx-audit-business-code";

#[derive(Clone)]
pub struct AuditService {
    database: Database,
    sender: Sender<AuditEvent>,
    dropped: Arc<AtomicU64>,
}

#[derive(Clone, Debug)]
pub struct AuditEvent {
    pub request_id: String,
    pub created_at: i64,
    pub client_ip: String,
    pub uid: Option<u64>,
    pub method: String,
    pub endpoint: String,
    pub scope: String,
    pub area: String,
    pub client_type: String,
    pub season_id: Option<u64>,
    pub ep_id: Option<u64>,
    pub keyword: String,
    pub http_status: u16,
    pub business_code: Option<i64>,
    pub duration_ms: u64,
    pub cache_hit: Option<bool>,
    pub upstream: String,
    pub blocked: bool,
    pub matched_rule_id: Option<i64>,
}

pub type AuditContext = Arc<Mutex<AuditEvent>>;

#[derive(Debug, Deserialize, Default)]
pub struct AuditQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub ip: Option<String>,
    pub uid: Option<u64>,
    pub endpoint: Option<String>,
    pub area: Option<String>,
    pub ep_id: Option<u64>,
    pub season_id: Option<u64>,
    pub blocked: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct AuditRecord {
    pub id: i64,
    pub request_id: String,
    pub created_at: i64,
    pub client_ip: String,
    pub uid: Option<u64>,
    pub method: String,
    pub endpoint: String,
    pub scope: String,
    pub area: String,
    pub client_type: String,
    pub season_id: Option<u64>,
    pub ep_id: Option<u64>,
    pub season_title: String,
    pub episode_title: String,
    pub keyword: String,
    pub http_status: u16,
    pub business_code: Option<i64>,
    pub duration_ms: u64,
    pub cache_hit: Option<bool>,
    pub upstream: String,
    pub blocked: bool,
    pub matched_rule_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AuditPage {
    pub total: u64,
    pub limit: u32,
    pub offset: u32,
    pub items: Vec<AuditRecord>,
}

#[derive(Debug, Serialize)]
pub struct AuditSummary {
    pub requests_24h: u64,
    pub blocked_24h: u64,
    pub errors_24h: u64,
    pub dropped_since_start: u64,
}

impl AuditEvent {
    pub fn new(method: &str, endpoint: &str, scope: &str, client_ip: &str) -> Self {
        Self {
            request_id: Alphanumeric.sample_string(&mut rand::rng(), 24),
            created_at: Utc::now().timestamp(),
            client_ip: client_ip.to_string(),
            uid: None,
            method: method.to_string(),
            endpoint: endpoint.to_string(),
            scope: scope.to_string(),
            area: String::new(),
            client_type: String::new(),
            season_id: None,
            ep_id: None,
            keyword: String::new(),
            http_status: 0,
            business_code: None,
            duration_ms: 0,
            cache_hit: None,
            upstream: String::new(),
            blocked: false,
            matched_rule_id: None,
        }
    }
}

impl AuditService {
    pub fn new(database: Database, capacity: usize) -> (Self, Receiver<AuditEvent>) {
        let (sender, receiver) = async_channel::bounded(capacity);
        (
            Self {
                database,
                sender,
                dropped: Arc::new(AtomicU64::new(0)),
            },
            receiver,
        )
    }

    pub fn record(&self, event: AuditEvent) {
        if self.sender.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn list(&self, query: &AuditQuery) -> Result<AuditPage, String> {
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let offset = query.offset.unwrap_or(0);
        let (where_clause, values) = audit_filters(query);

        self.database
            .with_connection(|connection| {
                let total_sql = format!(
                    "SELECT COUNT(*) FROM audit_events a LEFT JOIN episode_metadata m ON m.ep_id = a.ep_id {where_clause}"
                );
                let total: u64 = connection.query_row(
                    &total_sql,
                    params_from_iter(values.iter()),
                    |row| row.get(0),
                )?;

                let mut page_values = values.clone();
                page_values.push(Value::Integer(limit.into()));
                page_values.push(Value::Integer(offset.into()));
                let list_sql = format!(
                    r#"
                    SELECT a.id, a.request_id, a.created_at, a.client_ip, a.uid,
                           a.method, a.endpoint, a.scope, a.area, a.client_type,
                           COALESCE(a.season_id, m.season_id), a.ep_id,
                           COALESCE(m.season_title, ''), COALESCE(m.episode_title, ''),
                           a.keyword, a.http_status, a.business_code, a.duration_ms,
                           a.cache_hit, a.upstream, a.blocked, a.matched_rule_id
                    FROM audit_events a
                    LEFT JOIN episode_metadata m ON m.ep_id = a.ep_id
                    {where_clause}
                    ORDER BY a.created_at DESC, a.id DESC
                    LIMIT ? OFFSET ?
                    "#
                );
                let mut statement = connection.prepare(&list_sql)?;
                let items = statement
                    .query_map(params_from_iter(page_values.iter()), map_audit_record)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(AuditPage {
                    total,
                    limit,
                    offset,
                    items,
                })
            })
            .map_err(|error| error.to_string())
    }

    pub fn summary(&self) -> Result<AuditSummary, String> {
        let since = Utc::now().timestamp() - 24 * 60 * 60;
        self.database
            .with_connection(|connection| {
                let (requests, blocked, errors): (u64, u64, u64) = connection.query_row(
                    r#"
                    SELECT COUNT(*),
                           COALESCE(SUM(CASE WHEN blocked = 1 THEN 1 ELSE 0 END), 0),
                           COALESCE(SUM(CASE WHEN http_status >= 400 OR (business_code IS NOT NULL AND business_code != 0) THEN 1 ELSE 0 END), 0)
                    FROM audit_events WHERE created_at >= ?1
                    "#,
                    [since],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
                Ok(AuditSummary {
                    requests_24h: requests,
                    blocked_24h: blocked,
                    errors_24h: errors,
                    dropped_since_start: self.dropped.load(Ordering::Relaxed),
                })
            })
            .map_err(|error| error.to_string())
    }

    pub fn prune(&self, retention_days: u32) -> Result<usize, String> {
        let cutoff = Utc::now().timestamp() - i64::from(retention_days.max(1)) * 24 * 60 * 60;
        self.database
            .with_connection(|connection| {
                connection.execute("DELETE FROM audit_events WHERE created_at < ?1", [cutoff])
            })
            .map_err(|error| error.to_string())
    }
}

pub fn update_context(request: &actix_web::HttpRequest, update: impl FnOnce(&mut AuditEvent)) {
    use actix_web::HttpMessage;
    if let Some(context) = request.extensions().get::<AuditContext>() {
        if let Ok(mut event) = context.lock() {
            update(&mut event);
        }
    }
}

pub async fn run_worker(receiver: Receiver<AuditEvent>, database: Database) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .expect("audit metadata client must build");
    let enrichment_limit = Arc::new(Semaphore::new(4));

    while let Ok(event) = receiver.recv().await {
        let ep_id = event.ep_id;
        let request_id = event.request_id.clone();
        if let Err(error) = insert_event(&database, &event) {
            log::error!("failed to write audit event: {error}");
            continue;
        }

        if let Some(ep_id) = ep_id {
            let Ok(permit) = enrichment_limit.clone().try_acquire_owned() else {
                continue;
            };
            let database = database.clone();
            let client = client.clone();
            tokio::spawn(async move {
                let _permit = permit;
                if metadata_exists(&database, ep_id).unwrap_or(true) {
                    return;
                }
                if let Some(metadata) = fetch_episode_metadata(&client, ep_id).await {
                    if let Err(error) = save_metadata(&database, ep_id, &metadata) {
                        log::debug!("failed to save metadata for {request_id}: {error}");
                    }
                }
            });
        }
    }
}

pub fn business_code_from_body(body: &str) -> Option<i64> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    value["code"]
        .as_i64()
        .or_else(|| value["code"].as_str()?.parse().ok())
}

fn insert_event(database: &Database, event: &AuditEvent) -> rusqlite::Result<()> {
    database.with_connection(|connection| {
        connection.execute(
            r#"
            INSERT OR IGNORE INTO audit_events (
                request_id, created_at, client_ip, uid, method, endpoint, scope,
                area, client_type, season_id, ep_id, keyword, http_status,
                business_code, duration_ms, cache_hit, upstream, blocked, matched_rule_id
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19
            )
            "#,
            params![
                event.request_id,
                event.created_at,
                event.client_ip,
                event.uid,
                event.method,
                event.endpoint,
                event.scope,
                event.area,
                event.client_type,
                event.season_id,
                event.ep_id,
                event.keyword,
                event.http_status,
                event.business_code,
                event.duration_ms,
                event.cache_hit,
                event.upstream,
                event.blocked,
                event.matched_rule_id,
            ],
        )?;
        Ok(())
    })
}

#[derive(Debug)]
struct EpisodeMetadata {
    season_id: Option<u64>,
    season_title: String,
    episode_title: String,
}

async fn fetch_episode_metadata(client: &reqwest::Client, ep_id: u64) -> Option<EpisodeMetadata> {
    let response = client
        .get(format!(
            "https://api.bilibili.com/pgc/view/web/season?ep_id={ep_id}"
        ))
        .send()
        .await
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    if response["code"].as_i64()? != 0 {
        return None;
    }
    let result = &response["result"];
    let episode_title = result["episodes"]
        .as_array()?
        .iter()
        .find(|episode| episode["ep_id"].as_u64() == Some(ep_id))
        .map(|episode| {
            let title = episode["title"].as_str().unwrap_or("");
            let long_title = episode["long_title"].as_str().unwrap_or("");
            match (title.is_empty(), long_title.is_empty()) {
                (false, false) => format!("{title} {long_title}"),
                (false, true) => title.to_string(),
                _ => long_title.to_string(),
            }
        })
        .unwrap_or_default();
    Some(EpisodeMetadata {
        season_id: result["season_id"].as_u64(),
        season_title: result["season_title"]
            .as_str()
            .or_else(|| result["title"].as_str())
            .unwrap_or("")
            .to_string(),
        episode_title,
    })
}

fn metadata_exists(database: &Database, ep_id: u64) -> rusqlite::Result<bool> {
    database.with_connection(|connection| {
        connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM episode_metadata WHERE ep_id = ?1)",
            [ep_id],
            |row| row.get(0),
        )
    })
}

fn save_metadata(
    database: &Database,
    ep_id: u64,
    metadata: &EpisodeMetadata,
) -> rusqlite::Result<()> {
    database.with_connection(|connection| {
        connection.execute(
            r#"
            INSERT INTO episode_metadata (
                ep_id, season_id, season_title, episode_title, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(ep_id) DO UPDATE SET
                season_id = excluded.season_id,
                season_title = excluded.season_title,
                episode_title = excluded.episode_title,
                updated_at = excluded.updated_at
            "#,
            params![
                ep_id,
                metadata.season_id,
                metadata.season_title,
                metadata.episode_title,
                Utc::now().timestamp(),
            ],
        )?;
        Ok(())
    })
}

fn audit_filters(query: &AuditQuery) -> (String, Vec<Value>) {
    let mut filters = Vec::new();
    let mut values = Vec::new();
    macro_rules! add_filter {
        ($sql:expr, $value:expr) => {{
            filters.push($sql);
            values.push($value);
        }};
    }

    if let Some(value) = query.from {
        add_filter!("a.created_at >= ?", Value::Integer(value));
    }
    if let Some(value) = query.to {
        add_filter!("a.created_at <= ?", Value::Integer(value));
    }
    if let Some(value) = query.ip.as_ref().filter(|value| !value.is_empty()) {
        add_filter!("a.client_ip = ?", Value::Text(value.clone()));
    }
    if let Some(value) = query.uid {
        add_filter!("a.uid = ?", Value::Integer(value as i64));
    }
    if let Some(value) = query.endpoint.as_ref().filter(|value| !value.is_empty()) {
        add_filter!("a.endpoint LIKE ?", Value::Text(format!("%{value}%")));
    }
    if let Some(value) = query.area.as_ref().filter(|value| !value.is_empty()) {
        add_filter!("a.area = ?", Value::Text(value.clone()));
    }
    if let Some(value) = query.ep_id {
        add_filter!("a.ep_id = ?", Value::Integer(value as i64));
    }
    if let Some(value) = query.season_id {
        add_filter!(
            "COALESCE(a.season_id, m.season_id) = ?",
            Value::Integer(value as i64)
        );
    }
    if let Some(value) = query.blocked {
        add_filter!("a.blocked = ?", Value::Integer(i64::from(value)));
    }

    let clause = if filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    (clause, values)
}

fn map_audit_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRecord> {
    Ok(AuditRecord {
        id: row.get(0)?,
        request_id: row.get(1)?,
        created_at: row.get(2)?,
        client_ip: row.get(3)?,
        uid: row.get(4)?,
        method: row.get(5)?,
        endpoint: row.get(6)?,
        scope: row.get(7)?,
        area: row.get(8)?,
        client_type: row.get(9)?,
        season_id: row.get(10)?,
        ep_id: row.get(11)?,
        season_title: row.get(12)?,
        episode_title: row.get(13)?,
        keyword: row.get(14)?,
        http_status: row.get(15)?,
        business_code: row.get(16)?,
        duration_ms: row.get(17)?,
        cache_hit: row.get(18)?,
        upstream: row.get(19)?,
        blocked: row.get(20)?,
        matched_rule_id: row.get(21)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(request_id: &str, created_at: i64, endpoint: &str) -> AuditEvent {
        let mut event = AuditEvent::new("GET", endpoint, "other", "127.0.0.1");
        event.request_id = request_id.to_string();
        event.created_at = created_at;
        event.http_status = 200;
        event
    }

    #[test]
    fn audit_filters_and_retention_are_applied() {
        let database = Database::open(":memory:").unwrap();
        let (service, _receiver) = AuditService::new(database.clone(), 4);
        let now = Utc::now().timestamp();
        insert_event(&database, &event("old", now - 3 * 86_400, "/old")).unwrap();
        insert_event(&database, &event("new", now, "/playurltv")).unwrap();

        let page = service
            .list(&AuditQuery {
                endpoint: Some("playurltv".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].request_id, "new");
        assert_eq!(service.prune(1).unwrap(), 1);
    }

    #[test]
    fn queue_failures_increment_the_drop_counter() {
        let database = Database::open(":memory:").unwrap();
        let (service, receiver) = AuditService::new(database, 1);
        service.record(event("first", 1, "/first"));
        service.record(event("full", 2, "/second"));
        assert_eq!(service.summary().unwrap().dropped_since_start, 1);
        drop(receiver);
        service.record(event("closed", 3, "/third"));
        assert_eq!(service.summary().unwrap().dropped_since_start, 2);
    }

    #[test]
    fn business_code_supports_numeric_and_string_values() {
        assert_eq!(business_code_from_body(r#"{"code":-412}"#), Some(-412));
        assert_eq!(business_code_from_body(r#"{"code":"0"}"#), Some(0));
        assert_eq!(business_code_from_body("not-json"), None);
    }
}
