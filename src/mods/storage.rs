use rusqlite::{Connection, Result as SqliteResult};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct Database {
    inner: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> SqliteResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        }

        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;

        let database = Self {
            inner: Arc::new(Mutex::new(connection)),
        };
        database.migrate()?;
        Ok(database)
    }

    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> SqliteResult<T>,
    ) -> SqliteResult<T> {
        let connection = self
            .inner
            .lock()
            .map_err(|_| rusqlite::Error::InvalidQuery)?;
        operation(&connection)
    }

    fn migrate(&self) -> SqliteResult<()> {
        self.with_connection(|connection| {
            connection.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS admin_sessions (
                    token_hash TEXT PRIMARY KEY,
                    csrf_token TEXT NOT NULL,
                    expires_at INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_admin_sessions_expires
                    ON admin_sessions(expires_at);

                CREATE TABLE IF NOT EXISTS access_rules (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    subject_type TEXT NOT NULL,
                    subject_value TEXT NOT NULL,
                    action TEXT NOT NULL,
                    scope TEXT NOT NULL DEFAULT 'all',
                    reason TEXT NOT NULL DEFAULT '',
                    enabled INTEGER NOT NULL DEFAULT 1,
                    expires_at INTEGER,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    created_by TEXT NOT NULL DEFAULT 'admin'
                );
                CREATE INDEX IF NOT EXISTS idx_access_rules_active
                    ON access_rules(enabled, expires_at, scope);

                CREATE TABLE IF NOT EXISTS episode_metadata (
                    ep_id INTEGER PRIMARY KEY,
                    season_id INTEGER,
                    season_title TEXT NOT NULL DEFAULT '',
                    episode_title TEXT NOT NULL DEFAULT '',
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS audit_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    request_id TEXT NOT NULL UNIQUE,
                    created_at INTEGER NOT NULL,
                    client_ip TEXT NOT NULL,
                    uid INTEGER,
                    method TEXT NOT NULL,
                    endpoint TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    area TEXT NOT NULL DEFAULT '',
                    client_type TEXT NOT NULL DEFAULT '',
                    season_id INTEGER,
                    ep_id INTEGER,
                    keyword TEXT NOT NULL DEFAULT '',
                    http_status INTEGER NOT NULL,
                    business_code INTEGER,
                    duration_ms INTEGER NOT NULL,
                    cache_hit INTEGER,
                    upstream TEXT NOT NULL DEFAULT '',
                    blocked INTEGER NOT NULL DEFAULT 0,
                    matched_rule_id INTEGER,
                    FOREIGN KEY(matched_rule_id) REFERENCES access_rules(id)
                );
                CREATE INDEX IF NOT EXISTS idx_audit_events_created
                    ON audit_events(created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_audit_events_ip
                    ON audit_events(client_ip, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_audit_events_uid
                    ON audit_events(uid, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_audit_events_ep
                    ON audit_events(ep_id, created_at DESC);

                CREATE TABLE IF NOT EXISTS admin_actions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    created_at INTEGER NOT NULL,
                    action TEXT NOT NULL,
                    target TEXT NOT NULL DEFAULT '',
                    detail TEXT NOT NULL DEFAULT ''
                );
                "#,
            )?;
            Ok(())
        })
    }
}
