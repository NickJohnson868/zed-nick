use crate::{
    java_model,
    mapper_features::{self, Tag},
    navigation, sql_text,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::PathBuf};
use tree_sitter::{Node, Parser};
use url::Url;

pub fn expression(xml: &str, offset: usize) -> Option<(String, usize, usize)> {
    let prefix = xml.get(..offset)?;
    let begin = [prefix.rfind("#{"), prefix.rfind("${")]
        .into_iter()
        .flatten()
        .max()?
        + 2;
    let end = xml[begin..].find('}')? + begin;
    if offset > end {
        return None;
    }
    let raw = xml[begin..end].split(',').next()?.trim();
    let start = begin + xml[begin..end].find(raw)?;
    if raw.is_empty() || offset < start || offset > start + raw.len() {
        return None;
    }
    Some((raw.into(), start, start + raw.len()))
}
fn location(xml: &str, tag: &Tag, key: &str, uri: &str) -> Option<Value> {
    let a = tag.attrs.get(key)?;
    Some(
        json!({"uri":uri,"range":{"start":sql_text::position(xml,a.start),"end":sql_text::position(xml,a.end)}}),
    )
}
fn statement(tags: &[Tag], offset: usize) -> Option<usize> {
    tags.iter()
        .enumerate()
        .find(|(_, t)| {
            t.parent == Some(0)
                && matches!(t.name.as_str(), "select" | "insert" | "update" | "delete")
                && t.start <= offset
                && offset < t.end
        })
        .map(|(i, _)| i)
}
fn within(tags: &[Tag], mut index: Option<usize>, ancestor: usize) -> bool {
    while let Some(i) = index {
        if i == ancestor {
            return true;
        }
        index = tags[i].parent;
    }
    false
}

pub fn definitions(roots: &[PathBuf], xml: &str, offset: usize, uri: &str) -> Option<Vec<Value>> {
    let (path, _, _) = expression(xml, offset)?;
    let tags = mapper_features::tags(xml);
    let statement = statement(&tags, offset)?;
    let root = path.split(['.', '[']).next()?;
    // Innermost foreach scope wins over bind/Java names; never leak a loop variable outside its body.
    for tag in tags
        .iter()
        .rev()
        .filter(|t| t.name == "foreach" && t.start <= offset && offset < t.end)
    {
        for key in ["item", "index"] {
            if tag.value(key) == Some(root) {
                return Some(location(xml, tag, key, uri).into_iter().collect());
            }
        }
    }
    let namespace = tags.first()?.value("namespace")?;
    let mut binds = Vec::new();
    let mut visited = BTreeSet::new();
    bind_locations(
        roots,
        xml,
        uri,
        &tags,
        statement,
        root,
        offset,
        0,
        &mut visited,
        &mut binds,
    );
    if !binds.is_empty() {
        return Some(binds.into_iter().rev().take(1).collect());
    }
    let method = tags[statement].value("id")?;
    let model = java_model::load(roots, namespace)?;
    let methods = model.methods.get(method)?;
    if methods.len() != 1 {
        return Some(Vec::new());
    }
    let parameters = &methods[0];
    let matched = parameters.iter().enumerate().find(|(i, p)| {
        p.alias.as_deref() == Some(root)
            || p.alias.is_none() && p.name == root
            || format!("param{}", i + 1) == root
            || p.alias.is_none() && format!("arg{i}") == root
    });
    if let Some((_, parameter)) = matched {
        if let Some((_, field)) = path.split_once('.') {
            let kind = java_model::resolve(&model, &parameter.kind)?;
            return Some(field_definitions(roots, &kind, field));
        }
        return Some(parameter_locations(
            roots,
            namespace,
            method,
            &parameter.name,
        ));
    }
    if parameters.len() == 1 && parameters[0].alias.is_none() {
        let kind = java_model::resolve(&model, &parameters[0].kind)?;
        if java_model::fields(roots, &kind).is_some() {
            return Some(field_definitions(roots, &kind, &path));
        }
        // Single scalar parameters may use an arbitrary placeholder name in MyBatis.
        let simple = parameters[0].kind.rsplit('.').next().unwrap_or("");
        if matches!(
            simple,
            "String"
                | "Long"
                | "Integer"
                | "Boolean"
                | "Double"
                | "Float"
                | "Short"
                | "Byte"
                | "long"
                | "int"
                | "boolean"
                | "double"
                | "float"
                | "short"
                | "byte"
        ) {
            return Some(parameter_locations(
                roots,
                namespace,
                method,
                &parameters[0].name,
            ));
        }
    }
    Some(Vec::new())
}

#[allow(clippy::too_many_arguments)]
fn bind_locations(
    roots: &[PathBuf],
    xml: &str,
    uri: &str,
    tags: &[Tag],
    scope: usize,
    name: &str,
    before: usize,
    depth: usize,
    visited: &mut BTreeSet<String>,
    result: &mut Vec<Value>,
) {
    if depth > 8 {
        return;
    }
    let namespace = tags
        .first()
        .and_then(|t| t.value("namespace"))
        .unwrap_or("");
    for tag in tags
        .iter()
        .filter(|tag| within(tags, tag.parent, scope) && tag.start < before)
    {
        if tag.name == "bind"
            && tag.value("name") == Some(name)
            && !tags.iter().any(|loop_tag| {
                loop_tag.name == "foreach" && loop_tag.start < tag.start && tag.start < loop_tag.end
            })
            && let Some(value) = location(xml, tag, "name", uri)
        {
            result.push(value);
        }
        if tag.name != "include" {
            continue;
        }
        let Some(reference) = tag.value("refid") else {
            continue;
        };
        if reference.contains("${") {
            continue;
        }
        let (owner, id) = reference.rsplit_once('.').unwrap_or((namespace, reference));
        let key = format!("{owner}.{id}");
        if !visited.insert(key.clone()) {
            continue;
        }
        if owner == namespace {
            if let Some((index, fragment)) = tags
                .iter()
                .enumerate()
                .find(|(_, t)| t.parent == Some(0) && t.name == "sql" && t.value("id") == Some(id))
            {
                bind_locations(
                    roots,
                    xml,
                    uri,
                    tags,
                    index,
                    name,
                    fragment.end,
                    depth + 1,
                    visited,
                    result,
                );
            }
        } else {
            for root in roots {
                navigation::visit_sources(root, "xml", &mut |file| {
                    if let Ok(source) = fs::read_to_string(file)
                        && let Ok(target) = Url::from_file_path(file)
                    {
                        let other = mapper_features::tags(&source);
                        if other.first().and_then(|t| t.value("namespace")) == Some(owner)
                            && let Some((index, fragment)) =
                                other.iter().enumerate().find(|(_, t)| {
                                    t.parent == Some(0)
                                        && t.name == "sql"
                                        && t.value("id") == Some(id)
                                })
                        {
                            bind_locations(
                                roots,
                                &source,
                                target.as_str(),
                                &other,
                                index,
                                name,
                                fragment.end,
                                depth + 1,
                                visited,
                                result,
                            );
                        }
                    }
                });
            }
        }
        visited.remove(&key);
    }
}
fn field_definitions(roots: &[PathBuf], kind: &str, path: &str) -> Vec<Value> {
    let mut owner = kind.to_owned();
    let parts = path.split('.').collect::<Vec<_>>();
    for part in &parts[..parts.len() - 1] {
        let Some((fields, _)) = java_model::fields(roots, &owner) else {
            return Vec::new();
        };
        let Some(kind) = fields.get(*part) else {
            return Vec::new();
        };
        owner = kind.clone();
    }
    let mut result = Vec::new();
    for root in roots {
        navigation::visit_sources(root, "java", &mut |file| {
            if file
                .file_stem()
                .is_some_and(|n| n == owner.rsplit('.').next().unwrap_or(""))
                && let Ok(source) = fs::read_to_string(file)
                && let Ok(uri) = Url::from_file_path(file)
            {
                result.extend(navigation::field_locations(
                    &source,
                    &owner,
                    parts.last().unwrap(),
                    uri.as_str(),
                ));
            }
        });
    }
    result
}
fn parameter_locations(roots: &[PathBuf], owner: &str, method: &str, name: &str) -> Vec<Value> {
    let mut result = Vec::new();
    for root in roots {
        navigation::visit_sources(root, "java", &mut |file| {
            if file
                .file_stem()
                .is_none_or(|n| n != owner.rsplit('.').next().unwrap_or(""))
            {
                return;
            }
            let Ok(source) = fs::read_to_string(file) else {
                return;
            };
            let Ok(uri) = Url::from_file_path(file) else {
                return;
            };
            if navigation::java_locations(&source, owner, None, uri.as_str()).is_empty() {
                return;
            }
            let mut parser = Parser::new();
            if parser
                .set_language(&tree_sitter_java::LANGUAGE.into())
                .is_err()
            {
                return;
            }
            let Some(tree) = parser.parse(&source, None) else {
                return;
            };
            fn walk(
                node: Node<'_>,
                source: &str,
                method: &str,
                name: &str,
                uri: &str,
                result: &mut Vec<Value>,
            ) {
                if node.kind() == "method_declaration"
                    && node
                        .child_by_field_name("name")
                        .is_some_and(|n| &source[n.byte_range()] == method)
                {
                    if let Some(params) = node.child_by_field_name("parameters") {
                        let mut cursor = params.walk();
                        for parameter in params.named_children(&mut cursor) {
                            if let Some(n) = parameter.child_by_field_name("name")
                                && &source[n.byte_range()] == name
                            {
                                result.push(json!({"uri":uri,"range":{"start":sql_text::position(source,n.start_byte()),"end":sql_text::position(source,n.end_byte())}}));
                            }
                        }
                    }
                    return;
                }
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    walk(child, source, method, name, uri, result);
                }
            }
            walk(
                tree.root_node(),
                &source,
                method,
                name,
                uri.as_str(),
                &mut result,
            );
        });
    }
    result
}
