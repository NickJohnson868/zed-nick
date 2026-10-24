use postgres::{Client, config::SslMode};
use postgres_native_tls::MakeTlsConnector;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use url::Url;

#[derive(Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub active_connection: String,
    pub connections: BTreeMap<String, Connection>,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connection {
    pub application_yaml: Option<PathBuf>,
    pub yaml_section: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password_env: Option<String>,
    pub ssl_mode: String,
    pub schemas: Vec<String>,
    pub search_path: Vec<String>,
    pub schema_cache_seconds: u64,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            application_yaml: None,
            yaml_section: "spring.datasource".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            database: String::new(),
            user: String::new(),
            password_env: None,
            ssl_mode: "disable".into(),
            schemas: vec!["public".into()],
            search_path: Vec::new(),
            schema_cache_seconds: 300,
        }
    }
}

#[derive(Clone)]
pub struct Selected {
    pub name: String,
    pub root: PathBuf,
    pub connection: Connection,
    pub stamp: (Option<SystemTime>, Option<SystemTime>),
}

pub fn selected(root: &Path) -> Result<Option<Selected>, String> {
    let path = root.join(".zed/mybatis-jump.json");
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).map_err(|_| "无法读取 .zed/mybatis-jump.json".to_owned())?;
    let settings: Settings = serde_json::from_str(&text)
        .map_err(|_| "MyBatis 连接配置 JSON 无效，请检查配置字段和格式".to_owned())?;
    if settings.active_connection.is_empty() {
        return Ok(None);
    }
    let connection = settings
        .connections
        .get(&settings.active_connection)
        .ok_or("active_connection 没有对应的连接配置")?
        .clone();
    let yaml_stamp = connection
        .application_yaml
        .as_ref()
        .and_then(|p| fs::metadata(root.join(p)).ok()?.modified().ok());
    Ok(Some(Selected {
        name: settings.active_connection,
        root: root.into(),
        connection,
        stamp: (
            fs::metadata(path).ok().and_then(|m| m.modified().ok()),
            yaml_stamp,
        ),
    }))
}

#[derive(Clone, Default)]
pub struct Catalog {
    pub tables: BTreeMap<(String, String), BTreeMap<String, String>>,
    pub search_path: Vec<String>,
}

fn expand(value: &str) -> Result<String, String> {
    if let Some(value) = value.strip_prefix("${").and_then(|v| v.strip_suffix('}')) {
        let (key, fallback) = value
            .split_once(':')
            .map_or((value, None), |(k, v)| (k, Some(v)));
        std::env::var(key)
            .ok()
            .or_else(|| fallback.map(str::to_owned))
            .ok_or_else(|| format!("缺少环境变量 {key}"))
    } else {
        Ok(value.into())
    }
}

pub fn connect(selected: &Selected) -> Result<Client, String> {
    let source = &selected.connection;
    let mut config = postgres::Config::new();
    let mut host = source.host.clone();
    let mut port = source.port;
    let mut database = source.database.clone();
    let mut user = source.user.clone();
    let mut password = None;
    let mut jdbc_search_path = Vec::new();
    if let Some(path) = &source.application_yaml {
        let text = fs::read_to_string(selected.root.join(path))
            .map_err(|_| "无法读取指定的本地 dev YAML")?;
        let yaml: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&text).map_err(|_| "无法解析 dev YAML")?;
        let mut section = &yaml;
        for part in source.yaml_section.split('.') {
            section = &section[part];
        }
        let get = |key: &str| -> Result<String, String> {
            let value = section[key]
                .as_str()
                .ok_or_else(|| format!("dev YAML 数据源缺少 {key}"))?;
            expand(value)
        };
        let jdbc = get("url")?;
        let jdbc = jdbc
            .strip_prefix("jdbc:")
            .ok_or("只支持 PostgreSQL JDBC URL")?;
        let url = Url::parse(jdbc).map_err(|_| "dev YAML 中的 JDBC URL 无效")?;
        if url.scheme() != "postgresql" {
            return Err("当前数据源不是 PostgreSQL".into());
        }
        host = url.host_str().ok_or("JDBC URL 缺少主机")?.into();
        port = url.port().unwrap_or(5432);
        database = url.path().trim_start_matches('/').into();
        if let Some((_, path)) = url.query_pairs().find(|(key, _)| key == "currentSchema") {
            jdbc_search_path = path.split(',').map(str::to_owned).collect();
        }
        user = get("username")?;
        if section["password"].is_string() {
            password = Some(get("password")?);
        }
    }
    if let Some(key) = &source.password_env {
        password = Some(std::env::var(key).map_err(|_| format!("缺少密码环境变量 {key}"))?);
    }
    if database.is_empty() || user.is_empty() {
        return Err("连接配置缺少 database 或 user".into());
    }
    config
        .host(&host)
        .port(port)
        .dbname(&database)
        .user(&user)
        .application_name("zed-mybatis-jump")
        .connect_timeout(Duration::from_secs(3))
        .options(
            "-c default_transaction_read_only=on -c statement_timeout=3000 -c lock_timeout=1000",
        );
    if let Some(password) = password {
        config.password(password);
    }
    let mode = match source.ssl_mode.as_str() {
        "disable" => SslMode::Disable,
        "prefer" => SslMode::Prefer,
        "require" => SslMode::Require,
        _ => return Err("ssl_mode 只支持 disable、prefer、require".into()),
    };
    config.ssl_mode(mode);
    let tls = native_tls::TlsConnector::new().map_err(|_| "无法初始化 TLS")?;
    let mut client = config.connect(MakeTlsConnector::new(tls)).map_err(
        |_| "PostgreSQL 连接失败，请检查本地 dev 数据源、环境变量和服务状态（未输出凭据）",
    )?;
    let search_path = if source.search_path.is_empty() {
        &jdbc_search_path
    } else {
        &source.search_path
    };
    if !search_path.is_empty() {
        let path = search_path
            .iter()
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(",");
        client
            .query_one(
                "SELECT pg_catalog.set_config('search_path', $1, false)",
                &[&path],
            )
            .map_err(|_| "无法设置连接 search_path")?;
    }
    Ok(client)
}

pub fn schema_names(client: &mut Client) -> Result<Vec<String>, String> {
    client.query("SELECT nspname::text FROM pg_catalog.pg_namespace WHERE nspname NOT LIKE 'pg_%' AND nspname <> 'information_schema' ORDER BY nspname", &[])
        .map(|rows| rows.iter().map(|row| row.get(0)).collect())
        .map_err(|_| "读取 schema 列表失败".into())
}

pub fn catalog(client: &mut Client, connection: &Connection) -> Result<Catalog, String> {
    let mut result = Catalog {
        search_path: client
            .query_one("SELECT pg_catalog.current_schemas(false)", &[])
            .map_err(|_| "读取 search_path 失败")?
            .get(0),
        ..Default::default()
    };
    let rows = client.query(
        "SELECT n.nspname::text, c.relname::text, a.attname::text, pg_catalog.format_type(a.atttypid,a.atttypmod) \
         FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid \
         LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped \
         WHERE n.nspname=ANY($1) AND c.relkind IN ('r','p','v','m','f') ORDER BY n.nspname,c.relname,a.attnum LIMIT 200001",
        &[&connection.schemas],
    ).map_err(|_| "读取表字段元数据失败")?;
    if rows.len() > 200000 {
        return Err("表字段数量超过缓存上限，请缩小 schemas 范围".into());
    }
    for row in rows {
        let key: (String, String) = (row.get(0), row.get(1));
        let fields = result.tables.entry(key).or_default();
        if let Some(column) = row.get::<_, Option<String>>(2) {
            fields.insert(column, row.get::<_, Option<String>>(3).unwrap_or_default());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spring_defaults_do_not_split_jdbc_colons() {
        assert_eq!(
            expand("${ZED_MYBATIS_TEST_NOT_SET:jdbc:postgresql://localhost/db}").unwrap(),
            "jdbc:postgresql://localhost/db"
        );
        assert!(expand("${ZED_MYBATIS_TEST_NOT_SET}").is_err());
    }
}
