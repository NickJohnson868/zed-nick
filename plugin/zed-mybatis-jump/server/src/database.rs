use postgres::{Client, config::SslMode};
use postgres_native_tls::MakeTlsConnector;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub active_connection: String,
    pub connections: BTreeMap<String, Connection>,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connection {
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
    pub stamp: Option<SystemTime>,
}

pub fn selected(root: &Path) -> Result<Option<Selected>, String> {
    let path = root.join(".zed/mybatis-jump.json");
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).map_err(|_| "无法读取 .zed/mybatis-jump.json".to_owned())?;
    let settings: Settings = serde_json::from_str(&text)
        .map_err(|_| "MyBatis 连接配置无效：请显式配置 host、port、database、user 和 password_env，不再支持 application_yaml 或 yaml_section".to_owned())?;
    if settings.active_connection.is_empty() {
        return Ok(None);
    }
    let connection = settings
        .connections
        .get(&settings.active_connection)
        .ok_or("active_connection 没有对应的连接配置")?
        .clone();
    Ok(Some(Selected {
        name: settings.active_connection,
        root: root.into(),
        connection,
        stamp: fs::metadata(path).ok().and_then(|m| m.modified().ok()),
    }))
}

#[derive(Clone, Default)]
pub struct Catalog {
    pub tables: BTreeMap<(String, String), BTreeMap<String, String>>,
    pub search_path: Vec<String>,
}

pub fn connect(selected: &Selected) -> Result<Client, String> {
    let source = &selected.connection;
    let mut config = postgres::Config::new();
    let mut password = None;
    if let Some(key) = &source.password_env {
        password = Some(std::env::var(key).map_err(|_| format!("缺少密码环境变量 {key}"))?);
    }
    if source.host.trim().is_empty()
        || source.database.trim().is_empty()
        || source.user.trim().is_empty()
    {
        return Err("连接配置缺少 host、database 或 user".into());
    }
    config
        .host(&source.host)
        .port(source.port)
        .dbname(&source.database)
        .user(&source.user)
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
        |_| "PostgreSQL 连接失败，请检查用户配置的连接参数、密码环境变量和服务状态（未输出凭据）",
    )?;
    let search_path = &source.search_path;
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
    fn connection_uses_explicit_user_parameters() {
        let settings: Settings = serde_json::from_str(
            r#"{
            "active_connection":"local",
            "connections":{"local":{"host":"localhost","port":5544,
                "database":"example","user":"example_user","password_env":"MYBATIS_PG_PASSWORD",
                "search_path":["public"]}}
        }"#,
        )
        .expect("valid explicit connection");
        let connection = settings
            .connections
            .get("local")
            .expect("configured connection");
        assert_eq!(connection.host, "localhost");
        assert_eq!(connection.port, 5544);
        assert_eq!(connection.search_path, ["public"]);
        assert_eq!(
            connection.password_env.as_deref(),
            Some("MYBATIS_PG_PASSWORD")
        );
    }

    #[test]
    fn yaml_and_inline_password_are_rejected() {
        for input in [
            r#"{"application_yaml":"application-dev.yaml"}"#,
            r#"{"yaml_section":"spring.datasource"}"#,
            r#"{"password":"not-a-real-password"}"#,
        ] {
            assert!(serde_json::from_str::<Connection>(input).is_err());
        }
    }
}
