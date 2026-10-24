use crate::{java_model, navigation, sql_text};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

#[derive(Clone)]
pub struct Attribute {
    pub value: String,
    pub start: usize,
    pub end: usize,
}
#[derive(Clone)]
pub struct Tag {
    pub name: String,
    pub attrs: BTreeMap<String, Attribute>,
    pub parent: Option<usize>,
    pub start: usize,
    pub end: usize,
}
impl Tag {
    pub fn value(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(|a| a.value.as_str())
    }
}
pub fn tags(xml: &str) -> Vec<Tag> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<usize> = Vec::new();
    let mut tags: Vec<Tag> = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let (event, empty) = match reader.read_event() {
            Ok(Event::Start(e)) => (e, false),
            Ok(Event::Empty(e)) => (e, true),
            Ok(Event::End(_)) => {
                if let Some(index) = stack.pop() {
                    tags[index].end = reader.buffer_position() as usize;
                }
                continue;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => continue,
        };
        if tags.is_empty() && event.name().as_ref() != b"mapper" {
            return Vec::new();
        }
        let attrs = attributes(&event, xml);
        tags.push(Tag {
            name: String::from_utf8_lossy(event.name().as_ref()).into(),
            attrs,
            parent: stack.last().copied(),
            start,
            end: if empty {
                reader.buffer_position() as usize
            } else {
                xml.len()
            },
        });
        if !empty {
            stack.push(tags.len() - 1);
        }
    }
    tags
}
fn attributes(event: &BytesStart<'_>, xml: &str) -> BTreeMap<String, Attribute> {
    event
        .attributes()
        .flatten()
        .filter_map(|a| {
            let start = (a.value.as_ptr() as usize).checked_sub(xml.as_ptr() as usize)?;
            Some((
                String::from_utf8_lossy(a.key.as_ref()).into(),
                Attribute {
                    value: a.unescape_value().ok()?.into_owned(),
                    start,
                    end: start + a.value.len(),
                },
            ))
        })
        .collect()
}
fn owner(tags: &[Tag], mut index: Option<usize>) -> Option<String> {
    while let Some(i) = index {
        let tag = &tags[i];
        let kind = match tag.name.as_str() {
            "resultMap" => Some("type"),
            "association" => Some("javaType"),
            "collection" => Some("ofType"),
            _ => None,
        };
        if let Some(kind) = kind {
            return tag.value(kind).map(str::to_owned);
        }
        index = tag.parent;
    }
    None
}
fn diagnostic(xml: &str, attr: &Attribute, code: &str, message: String) -> Value {
    json!({"range":{"start":sql_text::position(xml,attr.start),"end":sql_text::position(xml,attr.end)},"severity":1,"source":"MyBatis","code":code,"message":message})
}
fn property(roots: &[PathBuf], owner: &str, path: &str) -> Option<bool> {
    let mut kind = owner.to_owned();
    for part in path.split('.') {
        if part.contains(['[', ']']) {
            return None;
        }
        let (fields, complete) = java_model::fields(roots, &kind)?;
        match fields.get(part) {
            Some(next) => kind = next.clone(),
            None => return if complete { Some(false) } else { None },
        }
    }
    Some(true)
}
pub fn diagnostics(roots: &[PathBuf], xml: &str) -> Vec<Value> {
    let tags = tags(xml);
    let mut result = Vec::new();
    for tag in &tags {
        if let (Some(attr), Some(owner)) = (tag.attrs.get("property"), owner(&tags, tag.parent))
            && property(roots, &owner, &attr.value) == Some(false)
        {
            result.push(diagnostic(
                xml,
                attr,
                "mybatis-property",
                format!("Java 属性不存在：{} · {}", attr.value, owner),
            ));
        }
    }
    result.extend(parameter_diagnostics(roots, xml, &tags));
    result.truncate(100);
    result
}
fn parameter_diagnostics(roots: &[PathBuf], xml: &str, tags: &[Tag]) -> Vec<Value> {
    let Some(mapper) = tags
        .first()
        .and_then(|t| t.value("namespace"))
        .and_then(|name| java_model::load(roots, name))
    else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for block in sql_text::blocks(xml).into_iter().take(100) {
        let Some(tag) = tags.iter().find(|t| {
            matches!(t.name.as_str(), "select" | "insert" | "update" | "delete")
                && t.start <= block.start
                && t.end >= block.end
        }) else {
            continue;
        };
        if block.dynamic && static_statement(xml, tag).is_none() {
            continue;
        }
        let Some(methods) = tag.value("id").and_then(|id| mapper.methods.get(id)) else {
            continue;
        };
        if methods.len() != 1 {
            continue;
        }
        let params = &methods[0];
        let mut names = BTreeMap::new();
        for (i, param) in params.iter().enumerate() {
            let kind = java_model::resolve(&mapper, &param.kind).unwrap_or_default();
            names.insert(format!("param{}", i + 1), kind.clone());
            if let Some(alias) = &param.alias {
                names.insert(alias.clone(), kind);
            } else {
                names.insert(param.name.clone(), kind.clone());
                names.insert(format!("arg{i}"), kind);
            }
        }
        let single = params.len() == 1 && params[0].alias.is_none();
        let single_kind = params
            .first()
            .and_then(|p| java_model::resolve(&mapper, &p.kind));
        let raw = &xml[block.start..block.end];
        let mut cursor = 0;
        while let Some(relative) = raw[cursor..].find("#{") {
            let begin = cursor + relative + 2;
            let Some(close) = raw[begin..].find('}') else {
                break;
            };
            let end = begin + close;
            cursor = end + 1;
            let prior = &raw[..begin];
            if prior
                .rfind("<!--")
                .is_some_and(|open| prior.rfind("-->").is_none_or(|close| close < open))
            {
                continue;
            }
            let expression = raw[begin..end].split(',').next().unwrap_or("").trim();
            if expression.is_empty() || expression.starts_with('_') {
                continue;
            }
            let (root, tail) = expression
                .split_once('.')
                .map_or((expression, None), |(a, b)| (a, Some(b)));
            let known = if let Some(kind) = names.get(root) {
                tail.and_then(|tail| property(roots, kind, tail))
            } else if single {
                single_kind
                    .as_ref()
                    .and_then(|kind| property(roots, kind, expression))
            } else if root.contains('[') {
                None
            } else {
                Some(false)
            };
            if known == Some(false) {
                let start = block.start + begin + raw[begin..end].find(expression).unwrap_or(0);
                let attr = Attribute {
                    value: expression.into(),
                    start,
                    end: start + expression.len(),
                };
                result.push(diagnostic(
                    xml,
                    &attr,
                    "mybatis-parameter",
                    format!("Mapper 参数或 DTO 属性不存在：{expression}"),
                ));
            }
        }
    }
    result
}

/// A resultMap column is checked only when every referencing SQL has a known static result shape.
pub fn column_diagnostics(
    xml: &str,
    columns: &BTreeMap<usize, Option<BTreeSet<String>>>,
) -> Vec<Value> {
    let tags = tags(xml);
    let mut result = Vec::new();
    let namespace = tags
        .first()
        .and_then(|t| t.value("namespace"))
        .unwrap_or("");
    for (index, map) in tags
        .iter()
        .enumerate()
        .filter(|(_, tag)| tag.name == "resultMap" && tag.parent == Some(0))
    {
        let Some(id) = map.value("id") else {
            continue;
        };
        // Nested maps, inherited definitions and prefixes require full MyBatis composition.
        if map.value("extends").is_some()
            || tags.iter().any(|t| {
                t.parent == Some(index)
                    && matches!(
                        t.name.as_str(),
                        "association" | "collection" | "discriminator"
                    )
            })
        {
            continue;
        }
        let qualified = format!("{namespace}.{id}");
        let mut shapes = Vec::new();
        let mut unknown = false;
        for statement in tags
            .iter()
            .filter(|tag| tag.parent == Some(0) && tag.name == "select")
        {
            if !statement.value("resultMap").is_some_and(|refs| {
                refs.split(',')
                    .any(|r| r.trim() == id || r.trim() == qualified)
            }) {
                continue;
            }
            let shape = columns
                .iter()
                .find(|(start, _)| statement.start <= **start && **start <= statement.end)
                .and_then(|(_, shape)| shape.as_ref());
            if let Some(shape) = shape {
                shapes.push(shape);
            } else {
                unknown = true;
            }
        }
        if unknown || shapes.is_empty() {
            continue;
        }
        for tag in tags
            .iter()
            .filter(|tag| tag.parent == Some(index) && matches!(tag.name.as_str(), "id" | "result"))
        {
            if let Some(column) = tag.attrs.get("column")
                && !column.value.contains(['{', ','])
                && !shapes.iter().any(|shape| {
                    shape
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&column.value))
                })
            {
                result.push(diagnostic(
                    xml,
                    column,
                    "mybatis-result-column",
                    format!(
                        "resultMap 列不在引用它的静态 SQL 返回结果中：{}",
                        column.value
                    ),
                ));
            }
        }
    }
    result
}

pub fn complete(roots: &[PathBuf], xml: &str, offset: usize) -> Option<Vec<Value>> {
    let tags = tags(xml);
    for tag in &tags {
        for (key, attr) in &tag.attrs {
            if !(attr.start..=attr.end).contains(&offset) {
                continue;
            }
            let prefix = &xml[attr.start..offset];
            let mut suggestions = BTreeMap::<String, String>::new();
            match key.as_str() {
                "property" => {
                    let kind = owner(&tags, tag.parent)?;
                    let (base, fragment) = prefix
                        .rsplit_once('.')
                        .map_or((None, prefix), |(base, tail)| (Some(base), tail));
                    let mut kind = kind;
                    if let Some(base) = base {
                        for part in base.split('.') {
                            kind = java_model::fields(roots, &kind)?.0.get(part)?.clone();
                        }
                    }
                    for (name, kind) in java_model::fields(roots, &kind)?.0 {
                        if name.to_lowercase().starts_with(&fragment.to_lowercase()) {
                            let value = base.map_or(name.clone(), |base| format!("{base}.{name}"));
                            suggestions.insert(value, kind);
                        }
                    }
                }
                "resultMap" | "refid" | "extends" => {
                    let target = if key == "refid" { "sql" } else { "resultMap" };
                    let segment = prefix.rsplit(',').next().unwrap_or("").trim_start();
                    if let Some((namespace, _)) = segment.rsplit_once('.') {
                        for id in reference_ids(roots, namespace, target) {
                            suggestions.insert(format!("{namespace}.{id}"), target.into());
                        }
                    } else {
                        for candidate in tags
                            .iter()
                            .filter(|t| t.name == target && t.parent == Some(0))
                        {
                            if let Some(id) = candidate.value("id") {
                                suggestions.insert(id.into(), target.into());
                            }
                        }
                    }
                }
                "type" | "resultType" | "parameterType" | "javaType" | "ofType" => {
                    for name in java_model::types(roots, prefix) {
                        suggestions.insert(name, "Java 类型".into());
                    }
                }
                _ => return None,
            }
            let (start, filter) = if matches!(key.as_str(), "resultMap" | "refid" | "extends") {
                let segment = prefix.rsplit(',').next().unwrap_or("");
                (offset - segment.trim_start().len(), segment.trim_start())
            } else {
                (attr.start, prefix)
            };
            let replace_end = if key == "resultMap" {
                xml[offset..attr.end]
                    .find(',')
                    .map_or(attr.end, |i| offset + i)
            } else {
                attr.end
            };
            return Some(suggestions.into_iter().filter(|(name,_)|name.to_lowercase().starts_with(&filter.to_lowercase())||matches!(key.as_str(),"type"|"resultType"|"parameterType"|"javaType"|"ofType")).take(100).map(|(name,detail)|json!({"label":name,"kind":if key=="property"{5}else if matches!(key.as_str(),"type"|"resultType"|"parameterType"|"javaType"|"ofType"){7}else{18},"detail":detail,"textEdit":{"range":{"start":sql_text::position(xml,start),"end":sql_text::position(xml,replace_end)},"newText":name}})).collect());
        }
    }
    None
}
type ReferenceCache =
    std::collections::HashMap<(Vec<PathBuf>, String, String), (std::time::Instant, Vec<String>)>;
fn reference_cache() -> &'static std::sync::Mutex<ReferenceCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<ReferenceCache>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}
pub fn invalidate_references() {
    reference_cache().lock().unwrap().clear();
}
fn reference_ids(roots: &[PathBuf], namespace: &str, kind: &str) -> Vec<String> {
    let key = (roots.to_vec(), namespace.into(), kind.into());
    if let Some((stamp, ids)) = reference_cache().lock().unwrap().get(&key)
        && stamp.elapsed() < std::time::Duration::from_secs(30)
    {
        return ids.clone();
    }
    let mut result = BTreeSet::new();
    for root in roots {
        navigation::visit_sources(root, "xml", &mut |path| {
            if let Ok(source) = fs::read_to_string(path) {
                let tags = tags(&source);
                if tags.first().and_then(|t| t.value("namespace")) == Some(namespace) {
                    for id in tags
                        .iter()
                        .filter(|t| t.name == kind && t.parent == Some(0))
                        .filter_map(|t| t.value("id"))
                    {
                        result.insert(id.to_owned());
                    }
                }
            }
        });
    }
    let result = result.into_iter().collect::<Vec<_>>();
    let mut cache = reference_cache().lock().unwrap();
    if cache.len() >= 32 {
        cache.clear();
    }
    cache.insert(key, (std::time::Instant::now(), result.clone()));
    result
}

/// Expand only fixed local includes. Unknown refs, include properties, cycles and dynamic tags are skipped.
pub fn static_statement(xml: &str, statement: &Tag) -> Option<String> {
    let all = tags(xml);
    fn body<'a>(xml: &'a str, tag: &Tag) -> Option<&'a str> {
        let raw = &xml[tag.start..tag.end];
        let start = raw.find('>')? + 1;
        let end = raw.rfind("</")?;
        (start <= end).then_some(&raw[start..end])
    }
    fn expand(xml: &str, source: &str, all: &[Tag], depth: usize) -> Option<String> {
        if depth > 8 || source.len() > 256 * 1024 {
            return None;
        }
        let wrapper = format!("<mapper><select id='expanded'>{source}</select></mapper>");
        let local = tags(&wrapper);
        let mut replacements = Vec::new();
        let base = "<mapper><select id='expanded'>".len();
        for include in local.iter().filter(|tag| tag.name == "include") {
            if include.attrs.len() != 1
                || local
                    .iter()
                    .any(|tag| tag.parent.is_some_and(|i| std::ptr::eq(&local[i], include)))
            {
                return None;
            }
            let reference = include.value("refid")?;
            if reference.contains("${") {
                return None;
            }
            let namespace = all.first()?.value("namespace").unwrap_or("");
            let id = if let Some((owner, id)) = reference.rsplit_once('.') {
                if owner != namespace {
                    return None;
                }
                id
            } else {
                reference
            };
            let fragment = all.iter().find(|tag| {
                tag.name == "sql" && tag.parent == Some(0) && tag.value("id") == Some(id)
            })?;
            let replacement = expand(xml, body(xml, fragment)?, all, depth + 1)?;
            replacements.push((include.start - base, include.end - base, replacement));
        }
        let mut expanded = source.to_owned();
        for (start, end, replacement) in replacements.into_iter().rev() {
            expanded.replace_range(start..end, &replacement);
            if expanded.len() > 256 * 1024 {
                return None;
            }
        }
        Some(expanded)
    }
    let content = expand(xml, body(xml, statement)?, &all, 0)?;
    let wrapper = format!("<mapper><select id='checked'>{content}</select></mapper>");
    let block = sql_text::blocks(&wrapper).into_iter().next()?;
    (!block.dynamic).then_some(block.sql)
}
