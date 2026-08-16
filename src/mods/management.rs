use super::{
    access_control::{AccessControlService, AccessRuleInput},
    audit::{AuditQuery, AuditService},
    config::save_biliconfig_atomic,
    storage::Database,
    types::{is_valid_domain_mapping_key, Area, BackgroundTaskType, BiliConfig},
};
use actix_web::{
    cookie::{time::Duration as CookieDuration, Cookie, SameSite},
    http::{header, StatusCode},
    web, HttpRequest, HttpResponse,
};
use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use async_channel::Sender;
use chrono::Utc;
use deadpool_redis::{redis::cmd, Pool};
use ipnet::IpNet;
use rand::distr::{Alphanumeric, SampleString};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Instant,
};

const SESSION_COOKIE: &str = "brpx_session";
const MASKED_VALUE: &str = "********";
const SESSION_LIFETIME_SECONDS: i64 = 24 * 60 * 60;

#[derive(Clone)]
pub struct AppState {
    pub redis_pool: Pool,
    pub channel: Arc<Sender<BackgroundTaskType>>,
    pub database: Database,
    pub access_control: AccessControlService,
    pub audit: AuditService,
    config: Arc<RwLock<Arc<BiliConfig>>>,
    config_path: Arc<PathBuf>,
    started_at: Instant,
}

#[derive(Debug)]
struct AdminSession {
    token_hash: String,
    csrf_token: String,
}

#[derive(Debug, Deserialize)]
struct PasswordPayload {
    password: String,
}

#[derive(Debug, Deserialize)]
struct PasswordChangePayload {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
struct ConfigPayload {
    config: Value,
}

impl AppState {
    pub fn new(
        config: BiliConfig,
        config_path: PathBuf,
        redis_pool: Pool,
        channel: Arc<Sender<BackgroundTaskType>>,
        database: Database,
        audit: AuditService,
    ) -> Self {
        let access_control = AccessControlService::new(database.clone());
        Self {
            redis_pool,
            channel,
            database,
            access_control,
            audit,
            config: Arc::new(RwLock::new(Arc::new(config))),
            config_path: Arc::new(config_path),
            started_at: Instant::now(),
        }
    }

    pub fn config_snapshot(&self) -> Arc<BiliConfig> {
        self.config
            .read()
            .expect("runtime config lock poisoned")
            .clone()
    }

    fn replace_config(&self, config: BiliConfig) {
        *self.config.write().expect("runtime config lock poisoned") = Arc::new(config);
    }

    pub fn resolve_client_ip(&self, request: &HttpRequest) -> IpAddr {
        let peer_ip = request
            .peer_addr()
            .map(|address| address.ip())
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let config = self.config_snapshot();
        if !is_trusted_proxy(peer_ip, &config.trusted_proxies) {
            return peer_ip;
        }

        if let Some(forwarded) = request
            .headers()
            .get("X-Forwarded-For")
            .and_then(|value| value.to_str().ok())
        {
            let chain = forwarded
                .split(',')
                .filter_map(|value| value.trim().parse::<IpAddr>().ok())
                .collect::<Vec<_>>();
            return chain
                .iter()
                .rev()
                .find(|address| !is_trusted_proxy(**address, &config.trusted_proxies))
                .copied()
                .or_else(|| chain.first().copied())
                .unwrap_or(peer_ip);
        }
        request
            .headers()
            .get("X-Real-IP")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .unwrap_or(peer_ip)
    }
}

pub fn configure_admin(config: &mut web::ServiceConfig) {
    config
        .route("/admin", web::get().to(admin_redirect))
        .route("/admin/", web::get().to(admin_page))
        .route("/admin/assets/lucide.min.js", web::get().to(lucide_script))
        .route(
            "/admin/assets/config-editor.js",
            web::get().to(config_editor_script),
        )
        .route(
            "/admin/api/bootstrap/status",
            web::get().to(bootstrap_status),
        )
        .route("/admin/api/bootstrap", web::post().to(bootstrap))
        .route("/admin/api/login", web::post().to(login))
        .route("/admin/api/session", web::get().to(session_status))
        .route("/admin/api/logout", web::post().to(logout))
        .route("/admin/api/dashboard", web::get().to(dashboard))
        .route("/admin/api/config", web::get().to(get_config))
        .route("/admin/api/config", web::put().to(update_config))
        .route("/admin/api/password", web::post().to(change_password))
        .route("/admin/api/rules", web::get().to(list_rules))
        .route("/admin/api/rules", web::post().to(create_rule))
        .route("/admin/api/rules/{id}", web::put().to(update_rule))
        .route("/admin/api/rules/{id}", web::delete().to(delete_rule))
        .route("/admin/api/audits", web::get().to(list_audits));
}

async fn admin_redirect() -> HttpResponse {
    HttpResponse::PermanentRedirect()
        .insert_header((header::LOCATION, "/admin/"))
        .finish()
}

async fn admin_page() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(include_str!("../html/admin.html"))
}

async fn lucide_script() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(include_str!("../html/lucide.min.js"))
}

async fn config_editor_script() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("application/javascript; charset=utf-8")
        .body(include_str!("../html/config-editor.js"))
}

async fn bootstrap_status(state: web::Data<AppState>) -> HttpResponse {
    match get_setting(&state.database, "admin_password_hash") {
        Ok(value) => ok(json!({ "initialized": value.is_some() })),
        Err(error) => internal_error(error),
    }
}

async fn bootstrap(
    request: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<PasswordPayload>,
) -> HttpResponse {
    match get_setting(&state.database, "admin_password_hash") {
        Ok(Some(_)) => return api_error(StatusCode::CONFLICT, "管理员已经初始化"),
        Ok(None) => {}
        Err(error) => return internal_error(error),
    }
    if let Err(error) = validate_password(&payload.password) {
        return api_error(StatusCode::BAD_REQUEST, &error);
    }
    let password_hash = match hash_password(&payload.password) {
        Ok(value) => value,
        Err(error) => return internal_error(error),
    };
    match insert_setting_if_absent(&state.database, "admin_password_hash", &password_hash) {
        Ok(true) => {}
        Ok(false) => return api_error(StatusCode::CONFLICT, "管理员已经初始化"),
        Err(error) => return internal_error(error),
    }
    record_admin_action(
        &state.database,
        "bootstrap",
        "admin",
        "initial administrator created",
    );
    create_session_response(&request, &state)
}

async fn login(
    request: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<PasswordPayload>,
) -> HttpResponse {
    let Some(password_hash) =
        get_setting(&state.database, "admin_password_hash").unwrap_or_default()
    else {
        return api_error(StatusCode::PRECONDITION_REQUIRED, "请先初始化管理员");
    };
    if !verify_password(&payload.password, &password_hash) {
        return api_error(StatusCode::UNAUTHORIZED, "密码错误");
    }
    create_session_response(&request, &state)
}

async fn session_status(request: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    match authenticate(&request, &state, false) {
        Ok(session) => ok(json!({
            "authenticated": true,
            "csrf_token": session.csrf_token,
        })),
        Err(_) => api_error(StatusCode::UNAUTHORIZED, "未登录"),
    }
}

async fn logout(request: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let session = match authenticate(&request, &state, true) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let _ = state.database.with_connection(|connection| {
        connection.execute(
            "DELETE FROM admin_sessions WHERE token_hash = ?1",
            [&session.token_hash],
        )
    });
    let mut cookie = Cookie::build(SESSION_COOKIE, "")
        .path("/admin")
        .http_only(true)
        .same_site(SameSite::Strict)
        .finish();
    cookie.make_removal();
    HttpResponse::Ok()
        .cookie(cookie)
        .json(json!({ "ok": true, "data": null }))
}

async fn dashboard(request: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, false) {
        return response;
    }
    let summary = match state.audit.summary() {
        Ok(value) => value,
        Err(error) => return internal_error(error),
    };
    let now = Utc::now().timestamp();
    let rules = state
        .access_control
        .list_rules()
        .map(|rules| {
            rules
                .into_iter()
                .filter(|rule| rule.enabled && rule.expires_at.is_none_or(|expiry| expiry > now))
                .count()
        })
        .unwrap_or(0);
    let redis_ok = match state.redis_pool.get().await {
        Ok(mut connection) => cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .is_ok(),
        Err(_) => false,
    };
    let config = state.config_snapshot();
    ok(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config_version": config.config_version,
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "redis_ok": redis_ok,
        "active_rules": rules,
        "audit": summary,
    }))
}

async fn get_config(request: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, false) {
        return response;
    }
    let mut value = match serde_json::to_value(state.config_snapshot().as_ref()) {
        Ok(value) => value,
        Err(error) => return internal_error(error.to_string()),
    };
    redact_secrets(&mut value, "");
    ok(json!({
        "config": value,
        "path": state.config_path.display().to_string(),
        "schema": config_schema(),
    }))
}

fn config_schema() -> Value {
    serde_json::from_str(include_str!("../html/config-schema.json"))
        .expect("embedded config schema must be valid JSON")
}

async fn update_config(
    request: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<ConfigPayload>,
) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, true) {
        return response;
    }
    let current = state.config_snapshot();
    let current_value = match serde_json::to_value(current.as_ref()) {
        Ok(value) => value,
        Err(error) => return internal_error(error.to_string()),
    };
    let mut next_value = payload.config.clone();
    preserve_masked_values(&current_value, &mut next_value);
    let Some(next_object) = next_value.as_object_mut() else {
        return api_error(StatusCode::BAD_REQUEST, "配置必须是 JSON 对象");
    };
    next_object.insert("config_version".to_string(), Value::from(7));

    let mut next: BiliConfig = match serde_json::from_value(next_value) {
        Ok(value) => value,
        Err(error) => return api_error(StatusCode::BAD_REQUEST, &format!("配置格式错误: {error}")),
    };
    if next.report_open {
        if let Err(error) = next.report_config.init() {
            return HttpResponse::BadRequest().json(json!({
                "ok": false,
                "error": "配置校验失败",
                "fields": [{ "field": "report_config", "message": error }],
            }));
        }
    }
    if let Err(errors) = validate_config(&next) {
        return HttpResponse::BadRequest().json(json!({
            "ok": false,
            "error": "配置校验失败",
            "fields": errors,
        }));
    }
    let restart_required = restart_required(current.as_ref(), &next);
    let backup_path = match save_biliconfig_atomic(&state.config_path, &next) {
        Ok(path) => path,
        Err(error) => return internal_error(format!("保存配置失败: {error}")),
    };
    state.replace_config(next);
    record_admin_action(
        &state.database,
        "config.update",
        &state.config_path.display().to_string(),
        if restart_required {
            "saved; restart required"
        } else {
            "saved; applied"
        },
    );
    ok(json!({
        "applied": true,
        "restart_required": restart_required,
        "backup_path": backup_path.display().to_string(),
    }))
}

async fn change_password(
    request: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<PasswordChangePayload>,
) -> HttpResponse {
    let session = match authenticate(&request, &state, true) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let current_hash = match get_setting(&state.database, "admin_password_hash") {
        Ok(Some(value)) => value,
        Ok(None) => return api_error(StatusCode::PRECONDITION_REQUIRED, "请先初始化管理员"),
        Err(error) => return internal_error(error),
    };
    if !verify_password(&payload.current_password, &current_hash) {
        return api_error(StatusCode::UNAUTHORIZED, "当前密码错误");
    }
    if let Err(error) = validate_password(&payload.new_password) {
        return api_error(StatusCode::BAD_REQUEST, &error);
    }
    if payload.current_password == payload.new_password {
        return api_error(StatusCode::BAD_REQUEST, "新密码不能与当前密码相同");
    }
    let new_hash = match hash_password(&payload.new_password) {
        Ok(value) => value,
        Err(error) => return internal_error(error),
    };
    if let Err(error) = state.database.with_connection(|connection| {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE settings SET value = ?1, updated_at = ?2 WHERE key = 'admin_password_hash'",
            params![new_hash, Utc::now().timestamp()],
        )?;
        transaction.execute(
            "DELETE FROM admin_sessions WHERE token_hash != ?1",
            [&session.token_hash],
        )?;
        transaction.commit()?;
        Ok(())
    }) {
        return internal_error(error.to_string());
    }
    record_admin_action(
        &state.database,
        "password.update",
        "admin",
        "password changed; other sessions invalidated",
    );
    ok(Value::Null)
}

async fn list_rules(request: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, false) {
        return response;
    }
    match state.access_control.list_rules() {
        Ok(rules) => ok(json!({ "items": rules })),
        Err(error) => internal_error(error),
    }
}

async fn create_rule(
    request: HttpRequest,
    state: web::Data<AppState>,
    payload: web::Json<AccessRuleInput>,
) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, true) {
        return response;
    }
    match state
        .access_control
        .create_rule(payload.into_inner(), "admin")
    {
        Ok(rule) => {
            record_admin_action(
                &state.database,
                "rule.create",
                &rule.id.to_string(),
                &rule.reason,
            );
            ok(json!(rule))
        }
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error),
    }
}

async fn update_rule(
    request: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<i64>,
    payload: web::Json<AccessRuleInput>,
) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, true) {
        return response;
    }
    match state
        .access_control
        .update_rule(path.into_inner(), payload.into_inner())
    {
        Ok(rule) => {
            record_admin_action(
                &state.database,
                "rule.update",
                &rule.id.to_string(),
                &rule.reason,
            );
            ok(json!(rule))
        }
        Err(error) if error == "rule not found" => api_error(StatusCode::NOT_FOUND, &error),
        Err(error) => api_error(StatusCode::BAD_REQUEST, &error),
    }
}

async fn delete_rule(
    request: HttpRequest,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, true) {
        return response;
    }
    let id = path.into_inner();
    match state.access_control.delete_rule(id) {
        Ok(true) => {
            record_admin_action(&state.database, "rule.delete", &id.to_string(), "");
            ok(Value::Null)
        }
        Ok(false) => api_error(StatusCode::NOT_FOUND, "rule not found"),
        Err(error) => internal_error(error),
    }
}

async fn list_audits(
    request: HttpRequest,
    state: web::Data<AppState>,
    query: web::Query<AuditQuery>,
) -> HttpResponse {
    if let Err(response) = authenticate(&request, &state, false) {
        return response;
    }
    match state.audit.list(&query) {
        Ok(page) => ok(json!(page)),
        Err(error) => internal_error(error),
    }
}

fn create_session_response(request: &HttpRequest, state: &AppState) -> HttpResponse {
    let raw_token = Alphanumeric.sample_string(&mut rand::rng(), 64);
    let csrf_token = Alphanumeric.sample_string(&mut rand::rng(), 40);
    let token_hash = digest(&raw_token);
    let now = Utc::now().timestamp();
    if let Err(error) = state.database.with_connection(|connection| {
        connection.execute("DELETE FROM admin_sessions WHERE expires_at <= ?1", [now])?;
        connection.execute(
            r#"
            INSERT INTO admin_sessions (token_hash, csrf_token, expires_at, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![token_hash, csrf_token, now + SESSION_LIFETIME_SECONDS, now,],
        )?;
        Ok(())
    }) {
        return internal_error(error.to_string());
    }
    let secure = request.connection_info().scheme() == "https";
    let cookie = Cookie::build(SESSION_COOKIE, raw_token)
        .path("/admin")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .max_age(CookieDuration::seconds(SESSION_LIFETIME_SECONDS))
        .finish();
    HttpResponse::Ok().cookie(cookie).json(json!({
        "ok": true,
        "data": {
            "authenticated": true,
            "csrf_token": csrf_token,
        }
    }))
}

fn authenticate(
    request: &HttpRequest,
    state: &AppState,
    csrf_required: bool,
) -> Result<AdminSession, HttpResponse> {
    let raw_token = request
        .cookie(SESSION_COOKIE)
        .map(|cookie| cookie.value().to_string())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "未登录"))?;
    let token_hash = digest(&raw_token);
    let now = Utc::now().timestamp();
    let csrf_token = state
        .database
        .with_connection(|connection| {
            connection
                .query_row(
                    r#"
                    SELECT csrf_token FROM admin_sessions
                    WHERE token_hash = ?1 AND expires_at > ?2
                    "#,
                    params![token_hash, now],
                    |row| row.get::<_, String>(0),
                )
                .optional()
        })
        .map_err(|error| internal_error(error.to_string()))?
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "会话已过期"))?;
    if csrf_required {
        let provided = request
            .headers()
            .get("X-BRPX-CSRF")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if provided != csrf_token {
            return Err(api_error(StatusCode::FORBIDDEN, "CSRF 校验失败"));
        }
    }
    Ok(AdminSession {
        token_hash,
        csrf_token,
    })
}

fn validate_password(password: &str) -> Result<(), String> {
    let character_count = password.chars().count();
    if character_count < 12 {
        return Err("管理员密码至少需要 12 个字符".to_string());
    }
    if character_count > 256 {
        return Err("管理员密码不能超过 256 个字符".to_string());
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt_source = Alphanumeric.sample_string(&mut rand::rng(), 24);
    let salt = SaltString::encode_b64(salt_source.as_bytes()).map_err(|error| error.to_string())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded)
        .ok()
        .and_then(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .ok()
        })
        .is_some()
}

fn get_setting(database: &Database, key: &str) -> Result<Option<String>, String> {
    database
        .with_connection(|connection| {
            connection
                .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()
        })
        .map_err(|error| error.to_string())
}

fn insert_setting_if_absent(database: &Database, key: &str, value: &str) -> Result<bool, String> {
    database
        .with_connection(|connection| {
            connection.execute(
                "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, Utc::now().timestamp()],
            )
        })
        .map(|changed| changed == 1)
        .map_err(|error| error.to_string())
}

fn record_admin_action(database: &Database, action: &str, target: &str, detail: &str) {
    if let Err(error) = database.with_connection(|connection| {
        connection.execute(
            "INSERT INTO admin_actions (created_at, action, target, detail) VALUES (?1, ?2, ?3, ?4)",
            params![Utc::now().timestamp(), action, target, detail],
        )?;
        Ok(())
    }) {
        log::warn!("failed to record admin action: {error}");
    }
}

fn redact_secrets(value: &mut Value, key: &str) {
    match value {
        Value::Object(object) => {
            for (child_key, child_value) in object {
                redact_secrets(child_value, child_key);
            }
        }
        Value::Array(array) => {
            for child in array {
                redact_secrets(child, key);
            }
        }
        Value::String(string) if is_secret_key(key) && !string.is_empty() => {
            *string = MASKED_VALUE.to_string();
        }
        _ => {}
    }
}

fn preserve_masked_values(current: &Value, next: &mut Value) {
    match (current, next) {
        (Value::Object(current), Value::Object(next)) => {
            for (key, next_value) in next {
                if let Some(current_value) = current.get(key) {
                    preserve_masked_values(current_value, next_value);
                }
            }
        }
        (Value::Array(current), Value::Array(next)) => {
            for (current_value, next_value) in current.iter().zip(next.iter_mut()) {
                preserve_masked_values(current_value, next_value);
            }
        }
        (Value::String(current), Value::String(next)) if next == MASKED_VALUE => {
            *next = current.clone()
        }
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "redis"
        || key == "api_sign"
        || key.contains("password")
        || key.contains("secret")
        || key.contains("token")
        || key.contains("access_key")
        || key.contains("private_key")
        || key.contains("proxy_url")
        || key.ends_with("_sign")
}

fn validate_config(config: &BiliConfig) -> Result<(), Vec<Value>> {
    let mut errors = Vec::new();
    if config.worker_num == 0 || config.worker_num > 128 {
        errors.push(json!({ "field": "worker_num", "message": "必须在 1 到 128 之间" }));
    }
    if config.http_port == 0 || config.https_port == 0 {
        errors.push(json!({ "field": "http_port", "message": "端口必须在 1 到 65535 之间" }));
    }
    if config.https_support && config.http_port == config.https_port {
        errors.push(json!({ "field": "https_port", "message": "HTTP 与 HTTPS 端口不能相同" }));
    }
    if !config.redis.starts_with("redis://") && !config.redis.starts_with("rediss://") {
        errors.push(json!({ "field": "redis", "message": "必须使用 redis:// 或 rediss://" }));
    }
    if !(1..=3650).contains(&config.audit_retention_days) {
        errors
            .push(json!({ "field": "audit_retention_days", "message": "必须在 1 到 3650 天之间" }));
    }
    for (index, proxy) in config.trusted_proxies.iter().enumerate() {
        if proxy.parse::<IpAddr>().is_err() && proxy.parse::<IpNet>().is_err() {
            errors.push(json!({
                "field": format!("trusted_proxies.{index}"),
                "message": "必须是 IP 或 CIDR"
            }));
        }
    }
    for (host, area) in &config.host_area_map {
        if !is_valid_domain_mapping_key(host) {
            errors.push(json!({
                "field": format!("host_area_map.{host}"),
                "message": "域名必须是不带协议、端口和路径的有效小写 DNS 主机名"
            }));
        }
        if Area::from_code(area).is_none() {
            errors.push(json!({
                "field": format!("host_area_map.{host}"),
                "message": "地区只能是 cn、hk、tw 或 th"
            }));
        }
    }
    for (field, entries) in [
        ("appsearch_remake", &config.appsearch_remake),
        ("websearch_remake", &config.websearch_remake),
    ] {
        for (host, serialized) in entries {
            if !is_valid_domain_mapping_key(host) {
                errors.push(json!({
                    "field": format!("{field}.{host}"),
                    "message": "域名必须是不带协议、端口和路径的有效小写 DNS 主机名"
                }));
            }
            match serde_json::from_str::<Value>(serialized) {
                Ok(Value::Object(_)) => {}
                Ok(_) => errors.push(json!({
                    "field": format!("{field}.{host}"),
                    "message": "JSON 根节点必须是对象"
                })),
                Err(error) => errors.push(json!({
                    "field": format!("{field}.{host}"),
                    "message": format!("JSON 无法解析: {error}")
                })),
            }
        }
    }
    let urls = [
        ("cn_app_playurl_api", &config.cn_app_playurl_api),
        ("tw_app_playurl_api", &config.tw_app_playurl_api),
        ("hk_app_playurl_api", &config.hk_app_playurl_api),
        ("th_app_playurl_api", &config.th_app_playurl_api),
        ("cn_web_playurl_api", &config.cn_web_playurl_api),
        ("tw_web_playurl_api", &config.tw_web_playurl_api),
        ("hk_web_playurl_api", &config.hk_web_playurl_api),
        ("th_web_playurl_api", &config.th_web_playurl_api),
        ("cn_tv_playurl_api", &config.cn_tv_playurl_api),
        ("tw_tv_playurl_api", &config.tw_tv_playurl_api),
        ("hk_tv_playurl_api", &config.hk_tv_playurl_api),
        ("cn_app_search_api", &config.cn_app_search_api),
        ("tw_app_search_api", &config.tw_app_search_api),
        ("hk_app_search_api", &config.hk_app_search_api),
        ("th_app_search_api", &config.th_app_search_api),
        ("cn_web_search_api", &config.cn_web_search_api),
        ("tw_web_search_api", &config.tw_web_search_api),
        ("hk_web_search_api", &config.hk_web_search_api),
        ("th_web_search_api", &config.th_web_search_api),
        ("th_app_season_api", &config.th_app_season_api),
        ("th_app_season_sub_api", &config.th_app_season_sub_api),
    ];
    for (field, url) in urls {
        if reqwest::Url::parse(url).is_err() {
            errors.push(json!({ "field": field, "message": "URL 无效" }));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn restart_required(current: &BiliConfig, next: &BiliConfig) -> bool {
    current.redis != next.redis
        || current.worker_num != next.worker_num
        || current.http_port != next.http_port
        || current.https_port != next.https_port
        || current.https_support != next.https_support
        || current.http2https_support != next.http2https_support
        || current.rate_limit_per_second != next.rate_limit_per_second
        || current.rate_limit_burst != next.rate_limit_burst
}

fn is_trusted_proxy(address: IpAddr, trusted_proxies: &[String]) -> bool {
    trusted_proxies.iter().any(|trusted| {
        trusted
            .parse::<IpAddr>()
            .map(|candidate| candidate == address)
            .or_else(|_| {
                trusted
                    .parse::<IpNet>()
                    .map(|network| network.contains(&address))
            })
            .unwrap_or(false)
    })
}

fn digest(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn ok(data: Value) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "ok": true, "data": data }))
}

fn api_error(status: StatusCode, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(json!({ "ok": false, "error": message }))
}

fn internal_error(error: impl Into<String>) -> HttpResponse {
    let error = error.into();
    log::error!("management API error: {error}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "管理服务内部错误")
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test as awtest, App};
    use std::collections::BTreeSet;

    fn test_state() -> AppState {
        let config: BiliConfig =
            serde_json::from_str(include_str!("../../config.example.json")).unwrap();
        let redis_pool = deadpool_redis::Config::from_url("redis://127.0.0.1:6379")
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .unwrap();
        let (channel, _receiver) = async_channel::bounded::<BackgroundTaskType>(1);
        let database = Database::open(":memory:").unwrap();
        let (audit, _audit_receiver) = AuditService::new(database.clone(), 1);
        AppState::new(
            config,
            PathBuf::from("config.json"),
            redis_pool,
            Arc::new(channel),
            database,
            audit,
        )
    }

    #[test]
    fn secret_redaction_and_restore_are_symmetric() {
        let original = json!({
            "redis": "redis://:password@127.0.0.1:6379",
            "nested": {
                "tg_bot_token": "token-\\\"value\\\\with-escapes",
                "proxy_url": "socks5://user:password@127.0.0.1:7890",
                "resign_api_sign": "shared-sign",
                "normal": "visible"
            }
        });
        let mut redacted = original.clone();
        redact_secrets(&mut redacted, "");
        assert_eq!(redacted["redis"], MASKED_VALUE);
        assert_eq!(redacted["nested"]["tg_bot_token"], MASKED_VALUE);
        assert_eq!(redacted["nested"]["proxy_url"], MASKED_VALUE);
        assert_eq!(redacted["nested"]["resign_api_sign"], MASKED_VALUE);
        preserve_masked_values(&original, &mut redacted);
        assert_eq!(redacted, original);
    }

    #[test]
    fn config_schema_covers_every_serialized_field() {
        let config: BiliConfig =
            serde_json::from_str(include_str!("../../config.example.json")).unwrap();
        let serialized = serde_json::to_value(config).unwrap();
        let expected = serialized
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let schema = config_schema();
        let mut actual = BTreeSet::new();
        for group in schema["groups"].as_array().unwrap() {
            assert!(!group["label"].as_str().unwrap_or("").is_empty());
            for field in group["fields"].as_array().unwrap() {
                let path = field["path"].as_str().unwrap();
                assert!(actual.insert(path.to_string()), "duplicate field: {path}");
                assert!(!field["label"].as_str().unwrap_or("").is_empty());
                assert!(!field["description"].as_str().unwrap_or("").is_empty());
                assert!(!field["effect"].as_str().unwrap_or("").is_empty());
            }
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn trusted_proxy_supports_exact_addresses_and_cidr() {
        let trusted = vec!["127.0.0.1".to_string(), "10.0.0.0/8".to_string()];
        assert!(is_trusted_proxy("127.0.0.1".parse().unwrap(), &trusted));
        assert!(is_trusted_proxy("10.2.3.4".parse().unwrap(), &trusted));
        assert!(!is_trusted_proxy("192.0.2.1".parse().unwrap(), &trusted));
    }

    #[test]
    fn host_area_map_rejects_invalid_hosts_and_areas() {
        let mut config: BiliConfig =
            serde_json::from_str(include_str!("../../config.example.json")).unwrap();
        config
            .host_area_map
            .insert("HK.EXAMPLE.COM:443".to_string(), "invalid".to_string());
        config
            .host_area_map
            .insert("new_key_1".to_string(), "cn".to_string());

        let errors = validate_config(&config).unwrap_err();
        assert!(errors.iter().any(|error| {
            error["field"] == "host_area_map.HK.EXAMPLE.COM:443"
                && error["message"].as_str().unwrap().contains("域名")
        }));
        assert!(errors.iter().any(|error| {
            error["field"] == "host_area_map.HK.EXAMPLE.COM:443"
                && error["message"].as_str().unwrap().contains("地区")
        }));
        assert!(errors.iter().any(|error| {
            error["field"] == "host_area_map.new_key_1"
                && error["message"].as_str().unwrap().contains("域名")
        }));
    }

    #[test]
    fn search_injection_requires_json_objects() {
        let mut config: BiliConfig =
            serde_json::from_str(include_str!("../../config.example.json")).unwrap();
        config
            .appsearch_remake
            .insert("broken.example.com".to_string(), "{".to_string());
        config
            .websearch_remake
            .insert("array.example.com".to_string(), "[]".to_string());

        let errors = validate_config(&config).unwrap_err();
        assert!(errors.iter().any(|error| {
            error["field"] == "appsearch_remake.broken.example.com"
                && error["message"].as_str().unwrap().contains("无法解析")
        }));
        assert!(errors.iter().any(|error| {
            error["field"] == "websearch_remake.array.example.com"
                && error["message"].as_str().unwrap().contains("必须是对象")
        }));
    }

    #[test]
    fn search_injection_rejects_invalid_domain_keys() {
        let mut config: BiliConfig =
            serde_json::from_str(include_str!("../../config.example.json")).unwrap();
        config
            .appsearch_remake
            .insert("HTTPS://EXAMPLE.COM/path".to_string(), "{}".to_string());

        let errors = validate_config(&config).unwrap_err();
        assert!(errors.iter().any(|error| {
            error["field"] == "appsearch_remake.HTTPS://EXAMPLE.COM/path"
                && error["message"].as_str().unwrap().contains("域名")
        }));
    }

    #[test]
    fn initial_setting_cannot_be_overwritten() {
        let database = Database::open(":memory:").unwrap();
        assert!(insert_setting_if_absent(&database, "admin_password_hash", "first").unwrap());
        assert!(!insert_setting_if_absent(&database, "admin_password_hash", "second").unwrap());
        assert_eq!(
            get_setting(&database, "admin_password_hash")
                .unwrap()
                .as_deref(),
            Some("first")
        );
    }

    #[test]
    fn password_length_counts_characters() {
        assert!(validate_password("十二个字符不足").is_err());
        assert!(validate_password("十二个字符的密码示例A1!").is_ok());
    }

    #[actix_web::test]
    async fn admin_mutations_require_authentication_and_csrf() {
        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(test_state()))
                .configure(configure_admin),
        )
        .await;
        let payload = json!({
            "subject_type": "ip",
            "subject_value": "203.0.113.8",
            "action": "deny",
            "scope": "all"
        });
        let unauthenticated = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/rules")
                .set_json(&payload)
                .to_request(),
        )
        .await;
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let bootstrap = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/bootstrap")
                .set_json(json!({ "password": "BRPX-test-password-1" }))
                .to_request(),
        )
        .await;
        let session_cookie = bootstrap.response().cookies().next().unwrap().into_owned();
        let without_csrf = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/rules")
                .cookie(session_cookie)
                .set_json(&payload)
                .to_request(),
        )
        .await;
        assert_eq!(without_csrf.status(), StatusCode::FORBIDDEN);
    }

    #[actix_web::test]
    async fn config_update_rejects_non_object_payloads() {
        let state = test_state();
        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_admin),
        )
        .await;
        let bootstrap = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/bootstrap")
                .set_json(json!({ "password": "BRPX-test-password-2" }))
                .to_request(),
        )
        .await;
        let session_cookie = bootstrap.response().cookies().next().unwrap().into_owned();
        let response: Value = awtest::read_body_json(bootstrap).await;
        let csrf = response["data"]["csrf_token"].as_str().unwrap();
        let invalid = awtest::call_service(
            &app,
            awtest::TestRequest::put()
                .uri("/admin/api/config")
                .cookie(session_cookie)
                .insert_header(("X-BRPX-CSRF", csrf))
                .set_json(json!({ "config": [] }))
                .to_request(),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[actix_web::test]
    async fn password_change_invalidates_other_sessions() {
        let app = awtest::init_service(
            App::new()
                .app_data(web::Data::new(test_state()))
                .configure(configure_admin),
        )
        .await;
        let bootstrap = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/bootstrap")
                .set_json(json!({ "password": "BRPX-old-password-1" }))
                .to_request(),
        )
        .await;
        let primary_cookie = bootstrap.response().cookies().next().unwrap().into_owned();
        let response: Value = awtest::read_body_json(bootstrap).await;
        let csrf = response["data"]["csrf_token"].as_str().unwrap();
        let secondary = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/login")
                .set_json(json!({ "password": "BRPX-old-password-1" }))
                .to_request(),
        )
        .await;
        let secondary_cookie = secondary.response().cookies().next().unwrap().into_owned();

        let changed = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/password")
                .cookie(primary_cookie)
                .insert_header(("X-BRPX-CSRF", csrf))
                .set_json(json!({
                    "current_password": "BRPX-old-password-1",
                    "new_password": "BRPX-new-password-2"
                }))
                .to_request(),
        )
        .await;
        assert_eq!(changed.status(), StatusCode::OK);

        let expired = awtest::call_service(
            &app,
            awtest::TestRequest::get()
                .uri("/admin/api/session")
                .cookie(secondary_cookie)
                .to_request(),
        )
        .await;
        assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
        let old_login = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/login")
                .set_json(json!({ "password": "BRPX-old-password-1" }))
                .to_request(),
        )
        .await;
        assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);
        let new_login = awtest::call_service(
            &app,
            awtest::TestRequest::post()
                .uri("/admin/api/login")
                .set_json(json!({ "password": "BRPX-new-password-2" }))
                .to_request(),
        )
        .await;
        assert_eq!(new_login.status(), StatusCode::OK);
    }
}
