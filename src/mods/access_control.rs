use super::storage::Database;
use chrono::Utc;
use ipnet::IpNet;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::IpAddr;

const PRIORITY_TOKEN: u16 = 1_000;
const PRIORITY_UID: u16 = 900;
const PRIORITY_IP: u16 = 800;
const PRIORITY_CIDR: u16 = 500;
const VALID_SCOPES: &[&str] = &[
    "all",
    "playurl",
    "tv",
    "search",
    "season",
    "subtitle",
    "accesskey",
];

#[derive(Clone)]
pub struct AccessControlService {
    database: Database,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccessRule {
    pub id: i64,
    pub subject_type: String,
    pub subject_value: String,
    pub action: String,
    pub scope: String,
    pub reason: String,
    pub enabled: bool,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub created_by: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AccessRuleInput {
    pub subject_type: String,
    pub subject_value: String,
    pub action: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct AccessDecision {
    pub allowed: bool,
    pub denied: bool,
    pub rule_id: Option<i64>,
    pub reason: String,
    pub expires_at: Option<i64>,
}

impl AccessControlService {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn list_rules(&self) -> Result<Vec<AccessRule>, String> {
        self.database
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    r#"
                    SELECT id, subject_type, subject_value, action, scope, reason,
                           enabled, expires_at, created_at, updated_at, created_by
                    FROM access_rules
                    ORDER BY enabled DESC, updated_at DESC, id DESC
                    "#,
                )?;
                let rules = statement
                    .query_map([], map_rule)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rules)
            })
            .map_err(|error| error.to_string())
    }

    pub fn get_rule(&self, id: i64) -> Result<Option<AccessRule>, String> {
        self.database
            .with_connection(|connection| {
                connection
                    .query_row(
                        r#"
                        SELECT id, subject_type, subject_value, action, scope, reason,
                               enabled, expires_at, created_at, updated_at, created_by
                        FROM access_rules WHERE id = ?1
                        "#,
                        [id],
                        map_rule,
                    )
                    .optional()
            })
            .map_err(|error| error.to_string())
    }

    pub fn create_rule(
        &self,
        input: AccessRuleInput,
        created_by: &str,
    ) -> Result<AccessRule, String> {
        let input = normalize_input(input)?;
        let now = Utc::now().timestamp();
        let id = self
            .database
            .with_connection(|connection| {
                connection.execute(
                    r#"
                    INSERT INTO access_rules (
                        subject_type, subject_value, action, scope, reason, enabled,
                        expires_at, created_at, updated_at, created_by
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)
                    "#,
                    params![
                        input.subject_type,
                        input.subject_value,
                        input.action,
                        input.scope,
                        input.reason,
                        input.enabled,
                        input.expires_at,
                        now,
                        created_by,
                    ],
                )?;
                Ok(connection.last_insert_rowid())
            })
            .map_err(|error| error.to_string())?;
        self.get_rule(id)?
            .ok_or_else(|| "created rule could not be read back".to_string())
    }

    pub fn update_rule(&self, id: i64, input: AccessRuleInput) -> Result<AccessRule, String> {
        let input = normalize_input(input)?;
        let updated = self
            .database
            .with_connection(|connection| {
                connection.execute(
                    r#"
                    UPDATE access_rules
                    SET subject_type = ?1, subject_value = ?2, action = ?3,
                        scope = ?4, reason = ?5, enabled = ?6, expires_at = ?7,
                        updated_at = ?8
                    WHERE id = ?9
                    "#,
                    params![
                        input.subject_type,
                        input.subject_value,
                        input.action,
                        input.scope,
                        input.reason,
                        input.enabled,
                        input.expires_at,
                        Utc::now().timestamp(),
                        id,
                    ],
                )
            })
            .map_err(|error| error.to_string())?;
        if updated == 0 {
            return Err("rule not found".to_string());
        }
        self.get_rule(id)?
            .ok_or_else(|| "updated rule could not be read back".to_string())
    }

    pub fn delete_rule(&self, id: i64) -> Result<bool, String> {
        self.database
            .with_connection(|connection| {
                let transaction = connection.unchecked_transaction()?;
                transaction.execute(
                    "UPDATE audit_events SET matched_rule_id = NULL WHERE matched_rule_id = ?1",
                    [id],
                )?;
                let changed =
                    transaction.execute("DELETE FROM access_rules WHERE id = ?1", [id])?;
                transaction.commit()?;
                Ok(changed)
            })
            .map(|changed| changed > 0)
            .map_err(|error| error.to_string())
    }

    pub fn evaluate(
        &self,
        client_ip: Option<IpAddr>,
        uid: Option<u64>,
        access_key: Option<&str>,
        scope: &str,
    ) -> Result<AccessDecision, String> {
        let now = Utc::now().timestamp();
        let rules = self
            .database
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    r#"
                    SELECT id, subject_type, subject_value, action, scope, reason,
                           enabled, expires_at, created_at, updated_at, created_by
                    FROM access_rules
                    WHERE enabled = 1 AND (expires_at IS NULL OR expires_at > ?1)
                    ORDER BY id ASC
                    "#,
                )?;
                let rules = statement
                    .query_map([now], map_rule)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rules)
            })
            .map_err(|error| error.to_string())?;

        let access_key_fingerprint = access_key.map(token_fingerprint);
        let mut selected: Option<(u16, bool, AccessRule)> = None;

        for rule in rules {
            if !scope_matches(&rule.scope, scope) {
                continue;
            }
            let priority = match rule.subject_type.as_str() {
                "token" if access_key_fingerprint.as_deref() == Some(&rule.subject_value) => {
                    PRIORITY_TOKEN
                }
                "uid"
                    if uid.map(|value| value.to_string()).as_deref()
                        == Some(&rule.subject_value) =>
                {
                    PRIORITY_UID
                }
                "ip" if client_ip.map(|value| value.to_string()).as_deref()
                    == Some(&rule.subject_value) =>
                {
                    PRIORITY_IP
                }
                "cidr" => match (client_ip, rule.subject_value.parse::<IpNet>()) {
                    (Some(ip), Ok(network)) if network.contains(&ip) => {
                        PRIORITY_CIDR + u16::from(network.prefix_len())
                    }
                    _ => continue,
                },
                _ => continue,
            };
            let is_deny = rule.action == "deny";
            let replace = selected
                .as_ref()
                .map(|(current_priority, current_deny, _)| {
                    priority > *current_priority
                        || (priority == *current_priority && is_deny && !*current_deny)
                })
                .unwrap_or(true);
            if replace {
                selected = Some((priority, is_deny, rule));
            }
        }

        Ok(match selected {
            Some((_, is_deny, rule)) => AccessDecision {
                allowed: !is_deny,
                denied: is_deny,
                rule_id: Some(rule.id),
                reason: rule.reason,
                expires_at: rule.expires_at,
            },
            None => AccessDecision::default(),
        })
    }
}

pub fn token_fingerprint(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn normalize_input(mut input: AccessRuleInput) -> Result<AccessRuleInput, String> {
    input.subject_type = input.subject_type.trim().to_ascii_lowercase();
    input.subject_value = input.subject_value.trim().to_string();
    input.action = input.action.trim().to_ascii_lowercase();
    input.scope = input.scope.trim().to_ascii_lowercase();
    input.reason = input.reason.trim().to_string();

    if !matches!(input.subject_type.as_str(), "uid" | "ip" | "cidr" | "token") {
        return Err("subject_type must be uid, ip, cidr, or token".to_string());
    }
    if !matches!(input.action.as_str(), "allow" | "deny") {
        return Err("action must be allow or deny".to_string());
    }
    if input.scope.is_empty() {
        input.scope = default_scope();
    }
    let mut scopes = input
        .scope
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .collect::<Vec<_>>();
    if scopes.is_empty() || scopes.iter().any(|scope| !VALID_SCOPES.contains(scope)) {
        return Err(format!(
            "scope must contain only: {}",
            VALID_SCOPES.join(", ")
        ));
    }
    scopes.dedup();
    if scopes.contains(&"all") && scopes.len() > 1 {
        return Err("scope 'all' cannot be combined with other scopes".to_string());
    }
    input.scope = scopes.join(",");
    if input.subject_value.is_empty() {
        return Err("subject_value cannot be empty".to_string());
    }

    input.subject_value = match input.subject_type.as_str() {
        "uid" => {
            let value = input
                .subject_value
                .parse::<u64>()
                .map_err(|_| "UID must be a positive integer".to_string())?;
            if value == 0 {
                return Err("UID must be a positive integer".to_string());
            }
            value.to_string()
        }
        "ip" => input
            .subject_value
            .parse::<IpAddr>()
            .map(|value| value.to_string())
            .map_err(|_| "invalid IP address".to_string())?,
        "cidr" => input
            .subject_value
            .parse::<IpNet>()
            .map(|value| value.to_string())
            .map_err(|_| "invalid CIDR network".to_string())?,
        "token" => {
            if input.subject_value.len() == 64
                && input
                    .subject_value
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            {
                input.subject_value.to_ascii_lowercase()
            } else {
                token_fingerprint(&input.subject_value)
            }
        }
        _ => unreachable!(),
    };

    if input
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now().timestamp())
    {
        return Err("expires_at must be in the future".to_string());
    }
    Ok(input)
}

fn scope_matches(rule_scope: &str, requested_scope: &str) -> bool {
    rule_scope
        .split(',')
        .map(str::trim)
        .any(|scope| scope == "all" || scope == requested_scope)
}

fn map_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccessRule> {
    Ok(AccessRule {
        id: row.get(0)?,
        subject_type: row.get(1)?,
        subject_value: row.get(2)?,
        action: row.get(3)?,
        scope: row.get(4)?,
        reason: row.get(5)?,
        enabled: row.get(6)?,
        expires_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        created_by: row.get(10)?,
    })
}

fn default_scope() -> String {
    "all".to_string()
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> AccessControlService {
        let database = Database::open(":memory:").unwrap();
        AccessControlService::new(database)
    }

    #[test]
    fn exact_uid_allow_overrides_cidr_deny() {
        let service = service();
        service
            .create_rule(
                AccessRuleInput {
                    subject_type: "cidr".to_string(),
                    subject_value: "10.0.0.0/8".to_string(),
                    action: "deny".to_string(),
                    scope: "all".to_string(),
                    reason: "network blocked".to_string(),
                    enabled: true,
                    expires_at: None,
                },
                "test",
            )
            .unwrap();
        service
            .create_rule(
                AccessRuleInput {
                    subject_type: "uid".to_string(),
                    subject_value: "42".to_string(),
                    action: "allow".to_string(),
                    scope: "playurl".to_string(),
                    reason: "trusted account".to_string(),
                    enabled: true,
                    expires_at: None,
                },
                "test",
            )
            .unwrap();

        let decision = service
            .evaluate(Some("10.1.2.3".parse().unwrap()), Some(42), None, "playurl")
            .unwrap();
        assert!(decision.allowed);
        assert!(!decision.denied);
    }

    #[test]
    fn deny_wins_when_specificity_is_equal() {
        let service = service();
        for action in ["allow", "deny"] {
            service
                .create_rule(
                    AccessRuleInput {
                        subject_type: "uid".to_string(),
                        subject_value: "7".to_string(),
                        action: action.to_string(),
                        scope: "all".to_string(),
                        reason: action.to_string(),
                        enabled: true,
                        expires_at: None,
                    },
                    "test",
                )
                .unwrap();
        }
        assert!(
            service
                .evaluate(None, Some(7), None, "search")
                .unwrap()
                .denied
        );
    }

    #[test]
    fn exact_ipv6_rule_overrides_host_cidr() {
        let service = service();
        service
            .create_rule(
                AccessRuleInput {
                    subject_type: "cidr".to_string(),
                    subject_value: "2001:db8::1/128".to_string(),
                    action: "deny".to_string(),
                    scope: "all".to_string(),
                    reason: String::new(),
                    enabled: true,
                    expires_at: None,
                },
                "test",
            )
            .unwrap();
        service
            .create_rule(
                AccessRuleInput {
                    subject_type: "ip".to_string(),
                    subject_value: "2001:db8::1".to_string(),
                    action: "allow".to_string(),
                    scope: "all".to_string(),
                    reason: String::new(),
                    enabled: true,
                    expires_at: None,
                },
                "test",
            )
            .unwrap();

        let decision = service
            .evaluate(Some("2001:db8::1".parse().unwrap()), None, None, "tv")
            .unwrap();
        assert!(decision.allowed);
    }

    #[test]
    fn input_validation_rejects_zero_uid_and_unknown_scope() {
        let service = service();
        let input = AccessRuleInput {
            subject_type: "uid".to_string(),
            subject_value: "0".to_string(),
            action: "deny".to_string(),
            scope: "all".to_string(),
            reason: String::new(),
            enabled: true,
            expires_at: None,
        };
        assert!(service.create_rule(input.clone(), "test").is_err());
        let mut invalid_scope = input;
        invalid_scope.subject_value = "1".to_string();
        invalid_scope.scope = "unknown".to_string();
        assert!(service.create_rule(invalid_scope, "test").is_err());
    }

    #[test]
    fn scope_and_expiry_limit_rule_matches() {
        let service = service();
        let rule = service
            .create_rule(
                AccessRuleInput {
                    subject_type: "ip".to_string(),
                    subject_value: "203.0.113.9".to_string(),
                    action: "deny".to_string(),
                    scope: "search".to_string(),
                    reason: String::new(),
                    enabled: true,
                    expires_at: Some(Utc::now().timestamp() + 60),
                },
                "test",
            )
            .unwrap();
        let address = Some("203.0.113.9".parse().unwrap());
        assert!(
            !service
                .evaluate(address, None, None, "playurl")
                .unwrap()
                .denied
        );
        assert!(
            service
                .evaluate(address, None, None, "search")
                .unwrap()
                .denied
        );

        service
            .database
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE access_rules SET expires_at = ?1 WHERE id = ?2",
                    params![Utc::now().timestamp() - 1, rule.id],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            !service
                .evaluate(address, None, None, "search")
                .unwrap()
                .denied
        );
    }

    #[test]
    fn deleting_a_matched_rule_preserves_the_audit_record() {
        let service = service();
        let rule = service
            .create_rule(
                AccessRuleInput {
                    subject_type: "ip".to_string(),
                    subject_value: "203.0.113.8".to_string(),
                    action: "deny".to_string(),
                    scope: "all".to_string(),
                    reason: String::new(),
                    enabled: true,
                    expires_at: None,
                },
                "test",
            )
            .unwrap();
        service
            .database
            .with_connection(|connection| {
                connection.execute(
                    r#"
                    INSERT INTO audit_events (
                        request_id, created_at, client_ip, method, endpoint, scope,
                        http_status, duration_ms, matched_rule_id
                    ) VALUES ('request-1', 1, '203.0.113.8', 'GET', '/test',
                              'other', 200, 1, ?1)
                    "#,
                    [rule.id],
                )?;
                Ok(())
            })
            .unwrap();

        assert!(service.delete_rule(rule.id).unwrap());
        let matched_rule_id = service
            .database
            .with_connection(|connection| {
                connection.query_row(
                    "SELECT matched_rule_id FROM audit_events WHERE request_id = 'request-1'",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
            })
            .unwrap();
        assert_eq!(matched_rule_id, None);
    }
}
