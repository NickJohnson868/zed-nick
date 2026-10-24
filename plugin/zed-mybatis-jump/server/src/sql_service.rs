use crate::{
    completion,
    database::{self, Catalog, Selected},
    mapper_features, sql_text,
};
use postgres::{Client, error::ErrorPosition};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

pub struct Document {
    pub text: String,
    pub version: i64,
}
struct Job {
    uri: String,
    text: String,
    version: i64,
    generation: u64,
    selected: Option<Selected>,
    roots: Vec<PathBuf>,
}
struct Response {
    uri: String,
    version: i64,
    generation: u64,
    diagnostics: Vec<Value>,
    catalog: Option<Arc<Catalog>>,
    notice: String,
}

fn issues(client: &mut Client, text: &str) -> Vec<Value> {
    let mut result = Vec::new();
    let mut shapes = BTreeMap::<usize, Option<BTreeSet<String>>>::new();
    for block in sql_text::blocks(text)
        .into_iter()
        .filter(|block| !block.dynamic && !block.sql.trim().is_empty())
        .take(100)
    {
        // 仅 Parse / Describe，绝不 Bind / Execute Mapper SQL。
        let prepared = client.prepare(&block.sql);
        shapes.insert(
            block.start,
            prepared.as_ref().ok().map(|statement| {
                statement
                    .columns()
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect()
            }),
        );
        if let Err(error) = prepared
            && let Some(error) = error.as_db_error()
            && matches!(error.code().code(), "42703" | "42P01" | "42601" | "42702")
            && let Some(ErrorPosition::Original(character)) = error.position()
        {
            let message = match error.code().code() {
                "42703" => "字段不存在",
                "42P01" => "表或关系不存在",
                "42702" => "字段归属不明确",
                _ => "SQL 语法错误",
            };
            result.push(sql_text::diagnostic(
                text,
                &block,
                *character as usize,
                &format!("{message}：{}", error.message()),
                error.code().code(),
            ));
        }
    }
    for statement in mapper_features::tags(text)
        .iter()
        .filter(|tag| {
            tag.name == "select" && tag.parent == Some(0) && tag.value("resultMap").is_some()
        })
        .take(100)
    {
        // Use the original statement position, even when include expansion changes byte offsets.
        if let Some(sql) = mapper_features::static_statement(text, statement) {
            let columns = client.prepare(&sql).ok().map(|statement| {
                statement
                    .columns()
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect()
            });
            shapes.insert(statement.start, columns);
        }
    }
    result.extend(mapper_features::column_diagnostics(text, &shapes));
    result
}

fn worker(rx: mpsc::Receiver<Job>, tx: mpsc::Sender<Response>) {
    let mut client: Option<Client> = None;
    let mut cached: Option<Arc<Catalog>> = None;
    let mut key = None;
    let mut refreshed = Instant::now();
    loop {
        let job = match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(job) => job,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                client = None;
                continue;
            }
            Err(_) => break,
        };
        let next_key = job
            .selected
            .as_ref()
            .map(|selected| (selected.root.clone(), selected.name.clone(), selected.stamp));
        if key != next_key {
            client = None;
            cached = None;
            key = next_key;
        }
        let mut local = mapper_features::diagnostics(&job.roots, &job.text);
        let outcome = (|| -> Result<(Vec<Value>, Arc<Catalog>), String> {
            let selected = job.selected.as_ref().ok_or("未启用数据库连接")?;
            if client.as_ref().is_none_or(Client::is_closed) {
                client = Some(database::connect(selected)?);
            }
            let database = client.as_mut().unwrap();
            if cached.is_none()
                || refreshed.elapsed()
                    > Duration::from_secs(selected.connection.schema_cache_seconds.clamp(30, 3600))
            {
                cached = Some(Arc::new(database::catalog(database, &selected.connection)?));
                refreshed = Instant::now();
            }
            Ok((
                issues(database, &job.text),
                cached.as_ref().unwrap().clone(),
            ))
        })();
        let response = match outcome {
            Ok((diagnostics, catalog)) => {
                local.extend(diagnostics);
                let notice = format!(
                    "MyBatis PostgreSQL [{}]：表字段校验已启用，缓存 {} 张表/视图",
                    job.selected.as_ref().map(|s| s.name.as_str()).unwrap_or(""),
                    catalog.tables.len()
                );
                Response {
                    uri: job.uri,
                    version: job.version,
                    generation: job.generation,
                    diagnostics: local,
                    catalog: Some(catalog),
                    notice,
                }
            }
            Err(error) => {
                client = None;
                Response {
                    uri: job.uri,
                    version: job.version,
                    generation: job.generation,
                    diagnostics: local,
                    catalog: None,
                    notice: if job.selected.is_some() {
                        format!("MyBatis PostgreSQL：{error}；暂不报告数据库字段错误")
                    } else {
                        "MyBatis：Java 属性与参数检查已启用".into()
                    },
                }
            }
        };
        if tx.send(response).is_err() {
            return;
        }
    }
}

pub struct SqlService {
    sender: mpsc::SyncSender<Job>,
    receiver: mpsc::Receiver<Response>,
    busy: bool,
    pending: HashMap<String, Instant>,
    selected: Option<Selected>,
    catalog: Option<Arc<Catalog>>,
    generation: u64,
    last_config: Instant,
    last_notice: String,
}

impl SqlService {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::channel();
        std::thread::spawn(move || worker(rx, result_tx));
        Self {
            sender: tx,
            receiver: result_rx,
            busy: false,
            pending: HashMap::new(),
            selected: None,
            catalog: None,
            generation: 0,
            last_config: Instant::now() - Duration::from_secs(2),
            last_notice: String::new(),
        }
    }
    pub fn changed(&mut self, uri: &str) {
        if uri.to_ascii_lowercase().ends_with(".xml") {
            self.pending
                .insert(uri.into(), Instant::now() + Duration::from_millis(450));
        }
    }
    pub fn closed(&mut self, uri: &str) {
        self.pending.remove(uri);
    }
    pub fn complete(&self, text: &str, offset: usize) -> Vec<Value> {
        self.catalog.as_ref().map_or_else(Vec::new, |catalog| {
            completion::complete(text, offset, catalog)
        })
    }

    pub fn tick(&mut self, roots: &[PathBuf], documents: &HashMap<String, Document>) -> Vec<Value> {
        let mut messages = Vec::new();
        if self.last_config.elapsed() >= Duration::from_secs(1) {
            self.last_config = Instant::now();
            let next = roots
                .iter()
                .find_map(|root| match database::selected(root) {
                    Ok(Some(selected)) => Some(Ok(selected)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                });
            let next = match next {
                Some(Ok(selected)) => Some(selected),
                Some(Err(error)) => {
                    if self.last_notice != error {
                        messages.push(json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":2,"message":error}}));
                        self.last_notice = error;
                    }
                    None
                }
                None => None,
            };
            let old = self.selected.as_ref().map(|s| (&s.root, &s.name, s.stamp));
            let new = next.as_ref().map(|s| (&s.root, &s.name, s.stamp));
            if old != new {
                self.generation += 1;
                self.selected = next;
                self.catalog = None;
                for uri in documents
                    .keys()
                    .filter(|uri| uri.to_ascii_lowercase().ends_with(".xml"))
                {
                    self.pending.insert(uri.clone(), Instant::now());
                    messages.push(publish(uri, documents[uri].version, Vec::new()));
                }
            }
        }
        while let Ok(response) = self.receiver.try_recv() {
            self.busy = false;
            if response.generation != self.generation {
                continue;
            }
            self.catalog = response.catalog;
            if response.notice != self.last_notice {
                messages.push(json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":if self.catalog.is_some(){3}else{2},"message":response.notice}}));
                self.last_notice = response.notice;
            }
            if documents
                .get(&response.uri)
                .is_some_and(|doc| doc.version == response.version)
            {
                messages.push(publish(
                    &response.uri,
                    response.version,
                    response.diagnostics,
                ));
            }
        }
        if !self.busy {
            let ready = self
                .pending
                .iter()
                .filter(|(_, due)| **due <= Instant::now())
                .min_by_key(|(_, due)| **due)
                .map(|(uri, _)| uri.clone());
            if let Some(uri) = ready {
                self.pending.remove(&uri);
                if let Some(doc) = documents.get(&uri)
                    && doc.text.len() <= 4 * 1024 * 1024
                {
                    let job = Job {
                        uri,
                        text: doc.text.clone(),
                        version: doc.version,
                        generation: self.generation,
                        selected: self.selected.clone(),
                        roots: roots.to_vec(),
                    };
                    self.busy = self.sender.try_send(job).is_ok();
                }
            }
        }
        messages
    }
}

pub fn publish(uri: &str, version: i64, diagnostics: Vec<Value>) -> Value {
    json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":uri,"version":version,"diagnostics":diagnostics}})
}

pub fn check_database(root: &std::path::Path) -> Result<Value, String> {
    let selected = database::selected(root)?.ok_or("没有启用数据库连接配置")?;
    let mut client = database::connect(&selected)?;
    let catalog = database::catalog(&mut client, &selected.connection)?;
    let xml = "<mapper><select id='valid'>SELECT relname FROM pg_catalog.pg_class</select><select id='invalid'>SELECT zed_mybatis_missing_column_92741 FROM pg_catalog.pg_class</select><select id='dynamic'>SELECT bad <if test='x'>FROM missing</if></select></mapper>";
    let diagnostics = issues(&mut client, xml);
    if diagnostics.len() != 1 || diagnostics[0]["code"] != "42703" {
        return Err("真实数据库字段校验自检失败".into());
    }
    let sample=catalog.tables.iter().find(|(_,columns)|!columns.is_empty()).map(|((schema,table),columns)|json!({"schema":schema,"table":table,"columns":columns.keys().take(10).collect::<Vec<_>>()}));
    Ok(
        json!({"schemas":database::schema_names(&mut client)?,"connection":selected.name,"tables":catalog.tables.len(),"columns":catalog.tables.values().map(|c|c.len()).sum::<usize>(),"readonly":client.query_one("SHOW default_transaction_read_only",&[]).map_err(|_|"检查只读状态失败")?.get::<_,String>(0),"undefined_column_check":true,"dynamic_sql_skipped":true,"sample":sample}),
    )
}
