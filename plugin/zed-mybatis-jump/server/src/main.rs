mod completion;
mod database;
mod highlighting;
mod java_model;
mod mapper_features;
mod navigation;
mod parameter_navigation;
mod sql_service;
mod sql_text;
mod xml_navigation;

use serde_json::{Value, json};
use sql_service::{Document, SqlService};
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};
use url::Url;

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length
        .filter(|n| *n <= 16 * 1024 * 1024)
        .ok_or_else(|| io::Error::other("invalid Content-Length"))?;
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

fn send(output: &mut impl Write, value: Value) -> io::Result<()> {
    let body = serde_json::to_vec(&value)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

fn file_path(uri: &Value) -> Option<PathBuf> {
    Url::parse(uri.as_str()?).ok()?.to_file_path().ok()
}

fn main() -> io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--check-db") {
        let root = std::env::args()
            .nth(2)
            .ok_or_else(|| io::Error::other("缺少项目目录"))?;
        let result =
            sql_service::check_database(std::path::Path::new(&root)).map_err(io::Error::other)?;
        println!("{}", result);
        return Ok(());
    }
    let (sender, receiver) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut input = io::BufReader::new(io::stdin().lock());
        while let Ok(Some(message)) = read_message(&mut input) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    let mut output = io::BufWriter::new(io::stdout().lock());
    let mut roots = Vec::new();
    let mut documents = HashMap::<String, Document>::new();
    let mut sql = SqlService::new();
    let mut shutdown = false;
    let mut definition_links = false;
    loop {
        let message = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !shutdown {
                    for message in sql.tick(&roots, &documents) {
                        send(&mut output, message)?;
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let method = message["method"].as_str().unwrap_or("");
        let params = &message["params"];
        let id = message.get("id").cloned();
        let result = match method {
            "initialize" => {
                definition_links =
                    params["capabilities"]["textDocument"]["definition"]["linkSupport"]
                        .as_bool()
                        .unwrap_or(false);
                if let Some(folders) = params["workspaceFolders"].as_array() {
                    roots.extend(folders.iter().filter_map(|f| file_path(&f["uri"])));
                }
                if roots.is_empty()
                    && let Some(path) = file_path(&params["rootUri"])
                {
                    roots.push(path);
                }
                json!({"capabilities":{"positionEncoding":"utf-16","textDocumentSync":{"openClose":true,"change":1,"save":true},"semanticTokensProvider":{"legend":{"tokenTypes":highlighting::TYPES,"tokenModifiers":[]},"full":true},"definitionProvider":true,"completionProvider":{"triggerCharacters":[".","\"","'"]}},"serverInfo":{"name":"MyBatis Jump","version":"0.2.0"}})
            }
            "shutdown" => {
                shutdown = true;
                Value::Null
            }
            "exit" => {
                std::process::exit(if shutdown { 0 } else { 1 });
            }
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["textDocument"]["text"].as_str(),
                ) {
                    documents.insert(
                        uri.into(),
                        Document {
                            text: text.into(),
                            version: params["textDocument"]["version"].as_i64().unwrap_or(0),
                        },
                    );
                    if let Some(path) = file_path(&json!(uri)) {
                        navigation::remember_source(&path);
                    }
                    sql.changed(uri);
                }
                Value::Null
            }
            "textDocument/didChange" => {
                if let (Some(uri), Some(text)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["contentChanges"]
                        .as_array()
                        .and_then(|changes| changes.last())
                        .and_then(|change| change["text"].as_str()),
                ) {
                    documents.insert(
                        uri.into(),
                        Document {
                            text: text.into(),
                            version: params["textDocument"]["version"].as_i64().unwrap_or(0),
                        },
                    );
                    sql.changed(uri);
                }
                Value::Null
            }
            "textDocument/didClose" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    documents.remove(uri);
                    sql.closed(uri);
                    if uri.to_ascii_lowercase().ends_with(".xml") {
                        send(&mut output, sql_service::publish(uri, 0, Vec::new()))?;
                    }
                }
                Value::Null
            }
            "textDocument/didSave" => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    if uri.to_ascii_lowercase().ends_with(".xml") {
                        mapper_features::invalidate_references();
                    }
                    if uri.to_ascii_lowercase().ends_with(".java") {
                        for xml in documents
                            .keys()
                            .filter(|uri| uri.to_ascii_lowercase().ends_with(".xml"))
                        {
                            sql.changed(xml);
                        }
                    }
                    if let Some(path) = file_path(&json!(uri)) {
                        navigation::remember_source(&path);
                    }
                    sql.changed(uri);
                }
                Value::Null
            }
            "textDocument/semanticTokens/full" if !shutdown => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                if uri.to_ascii_lowercase().ends_with(".xml") {
                    documents
                        .get(uri)
                        .map_or(json!({"data":[]}), |doc| highlighting::tokens(&doc.text))
                } else {
                    json!({"data":[]})
                }
            }
            "textDocument/completion" if !shutdown => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                if uri.to_ascii_lowercase().ends_with(".xml")
                    && let Some(doc) = documents.get(uri)
                    && let Some(offset) = navigation::byte_offset(
                        &doc.text,
                        params["position"]["line"].as_u64().unwrap_or(0) as usize,
                        params["position"]["character"].as_u64().unwrap_or(0) as usize,
                    )
                {
                    json!(
                        mapper_features::complete(&roots, &doc.text, offset)
                            .unwrap_or_else(|| sql.complete(&doc.text, offset))
                    )
                } else {
                    json!([])
                }
            }
            "textDocument/definition" if !shutdown => {
                let text = params["textDocument"]["uri"]
                    .as_str()
                    .and_then(|uri| documents.get(uri))
                    .map(|doc| doc.text.as_str());
                let target = text.and_then(|text| {
                    let offset = navigation::byte_offset(
                        text,
                        params["position"]["line"].as_u64()? as usize,
                        params["position"]["character"].as_u64()? as usize,
                    )?;
                    let uri = params["textDocument"]["uri"].as_str()?;
                    if uri.to_ascii_lowercase().ends_with(".xml") {
                        navigation::xml_target(text, offset)
                            .map(|(namespace, method)| (namespace, method, true))
                    } else {
                        navigation::mapper_method(text, offset)
                            .map(|(namespace, method)| (namespace, Some(method), false))
                    }
                });
                let xml_reference = text.and_then(|text| {
                    let uri = params["textDocument"]["uri"].as_str()?;
                    if !uri.to_ascii_lowercase().ends_with(".xml") {
                        return None;
                    }
                    let offset = navigation::byte_offset(
                        text,
                        params["position"]["line"].as_u64()? as usize,
                        params["position"]["character"].as_u64()? as usize,
                    )?;
                    parameter_navigation::definitions(&roots, text, offset, uri)
                        .or_else(|| xml_navigation::definitions(&roots, text, offset, uri))
                });
                let locations = if let Some(results) = xml_reference {
                    json!(results)
                } else if let Some((namespace, method, reverse)) = target {
                    json!(navigation::definitions(
                        &roots,
                        &namespace,
                        method.as_deref(),
                        reverse
                    ))
                } else {
                    json!([])
                };
                let origin = text.and_then(|text| {
                    if !definition_links
                        || !params["textDocument"]["uri"]
                            .as_str()?
                            .to_ascii_lowercase()
                            .ends_with(".xml")
                    {
                        return None;
                    }
                    let offset = navigation::byte_offset(
                        text,
                        params["position"]["line"].as_u64()? as usize,
                        params["position"]["character"].as_u64()? as usize,
                    )?;
                    parameter_navigation::expression(text,offset).map(|(_,start,end)|json!({"start":sql_text::position(text,start),"end":sql_text::position(text,end)})).or_else(||xml_navigation::origin_range(text, offset))
                });
                if let Some(origin) = origin {
                    json!(locations.as_array().unwrap().iter().map(|location|json!({"originSelectionRange":origin,"targetUri":location["uri"],"targetRange":location["range"],"targetSelectionRange":location["range"]})).collect::<Vec<_>>())
                } else {
                    locations
                }
            }
            _ if id.is_some() => {
                send(
                    &mut output,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not supported"}}),
                )?;
                continue;
            }
            _ => Value::Null,
        };
        if let Some(id) = id {
            send(
                &mut output,
                json!({"jsonrpc":"2.0","id":id,"result":result}),
            )?;
        }
        if !shutdown {
            for message in sql.tick(&roots, &documents) {
                send(&mut output, message)?;
            }
        }
    }
    Ok(())
}
