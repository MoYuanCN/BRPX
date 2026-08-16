use super::types::{BiliConfig, BiliRuntime};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};
pub fn init_biliconfig() -> BiliConfig {
    let config_path = match active_config_path() {
        Ok(path) => path,
        Err(error) => {
            println!("{error}");
            std::process::exit(78);
        }
    };
    let mut config = match load_biliconfig(&config_path) {
        Ok(value) => value,
        Err(value) => {
            println!("{value}");
            std::process::exit(78);
        }
    };
    let report_config = &mut config.report_config;
    if config.report_open {
        match report_config.init() {
            Ok(_) => (),
            Err(value) => {
                println!("{}", value);
                config.report_open = false;
            }
        }
    }
    config
}

pub fn active_config_path() -> Result<PathBuf, String> {
    ["config.json", "config.yml", "config.yaml"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .ok_or_else(|| "[error] 无配置文件，请复制 config.example.json 为 config.json".to_string())
}

fn load_biliconfig(path: &Path) -> Result<BiliConfig, String> {
    let config_file = File::open(path)
        .map_err(|error| format!("[error] 配置文件 {} 打开失败: {error}", path.display()))?;
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => serde_json::from_reader(config_file)
            .map_err(|error| format!("[error] JSON 配置解析失败: {error}")),
        Some("yml" | "yaml") => serde_yaml::from_reader(config_file)
            .map_err(|error| format!("[error] YAML 配置解析失败: {error}")),
        _ => Err("[error] 配置文件仅支持 json、yml 或 yaml".to_string()),
    }
}

pub fn save_biliconfig_atomic(path: &Path, config: &BiliConfig) -> Result<PathBuf, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or_else(|| "配置文件缺少扩展名".to_string())?;
    let serialized = match extension {
        "json" => serde_json::to_string_pretty(config).map_err(|error| error.to_string())?,
        "yml" | "yaml" => serde_yaml::to_string(config).map_err(|error| error.to_string())?,
        _ => return Err("配置文件仅支持 json、yml 或 yaml".to_string()),
    };
    let temporary_path = path.with_extension(format!("{extension}.tmp"));
    let backup_path = path.with_extension(format!("{extension}.bak"));

    let mut temporary = File::create(&temporary_path).map_err(|error| error.to_string())?;
    temporary
        .write_all(serialized.as_bytes())
        .and_then(|_| temporary.sync_all())
        .map_err(|error| error.to_string())?;

    if backup_path.exists() {
        fs::remove_file(&backup_path).map_err(|error| error.to_string())?;
    }
    fs::rename(path, &backup_path).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::rename(&backup_path, path);
        return Err(format!("替换配置文件失败: {error}"));
    }
    Ok(backup_path)
}

pub async fn prepare_before_start(bili_runtime: BiliRuntime<'_>) {
    // set resign_info
    if bili_runtime.config.cn_resign_info.access_key != "".to_owned() {
        bili_runtime
            .redis_set("a11101", &bili_runtime.config.cn_resign_info.to_json(), 0)
            .await;
    }

    if bili_runtime.config.th_resign_info.access_key != "".to_owned() {
        bili_runtime
            .redis_set("a41101", &bili_runtime.config.th_resign_info.to_json(), 0)
            .await;
    }
}

pub fn load_sslconfig() -> Result<rustls::ServerConfig, Box<dyn std::error::Error>> {
    use rustls::ServerConfig;
    use std::io::BufReader;

    let mut cert_file = BufReader::new(File::open("certificates/fullchain.pem")?);
    let mut private_key_file = BufReader::new(File::open("certificates/privkey.pem")?);

    let cert_chain = rustls_pemfile::certs(&mut cert_file).collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut private_key_file)?
        .ok_or("no private keys found in certificates/privkey.pem")?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;

    Ok(config)
}

pub async fn update_biliconfig() -> Result<bool, Box<dyn std::error::Error>> {
    use tokio::fs;
    let path = active_config_path().map_err(std::io::Error::other)?;
    let raw = fs::read_to_string(&path).await?;
    let mut config: serde_json::Value = match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(&raw)?,
        Some("yml" | "yaml") => {
            let yaml: serde_yaml::Value = serde_yaml::from_str(&raw)?;
            serde_json::to_value(yaml)?
        }
        _ => return Err("unsupported config format".into()),
    };
    let is_updated = migrate_config_value(&mut config);
    if !is_updated {
        return Ok(false);
    }

    let migrated = serde_json::from_value::<BiliConfig>(config)?;
    save_biliconfig_atomic(&path, &migrated).map_err(std::io::Error::other)?;
    Ok(true)
}

fn migrate_config_value(config: &mut serde_json::Value) -> bool {
    let Some(object) = config.as_object_mut() else {
        return false;
    };
    let version = object
        .get("config_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    if version >= 5 {
        return false;
    }

    if !object.contains_key("http_port") {
        if let Some(port) = object.get("port").cloned() {
            object.insert("http_port".to_string(), port);
        }
    }
    if !object.contains_key("worker_num") {
        if let Some(worker_num) = object.get("woker_num").cloned() {
            object.insert("worker_num".to_string(), worker_num);
        }
    }
    if !object.contains_key("resign_from_api_open") {
        if let Some(policy) = object.get("resign_api_policy").cloned() {
            object.insert("resign_from_api_open".to_string(), policy);
        }
    }
    for deprecated in [
        "port",
        "woker_num",
        "resign_api_policy",
        "local_wblist",
        "blacklist_config",
    ] {
        object.remove(deprecated);
    }
    object.insert("config_version".to_string(), serde_json::Value::from(5));
    true
}

#[cfg(test)]
mod tests {
    use super::migrate_config_value;

    #[test]
    fn migration_preserves_correct_worker_num() {
        let mut value = serde_json::json!({
            "config_version": 2,
            "port": 2662,
            "worker_num": 8,
            "woker_num": 4
        });
        assert!(migrate_config_value(&mut value));
        assert_eq!(value["http_port"], 2662);
        assert_eq!(value["worker_num"], 8);
        assert_eq!(value["config_version"], 5);
        assert!(value.get("port").is_none());
        assert!(value.get("woker_num").is_none());
    }
}
