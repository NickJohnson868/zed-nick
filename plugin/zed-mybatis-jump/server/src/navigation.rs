use quick_xml::{Reader, events::Event};
use serde_json::{Value, json};
use std::{fs, path::Path};
use tree_sitter::{Node, Parser};
use url::Url;

/// LSP 使用 UTF-16 列号；Tree-sitter 与 XML 定位使用 UTF-8 字节偏移。
pub fn byte_offset(text: &str, line: usize, column: usize) -> Option<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += text.get(start..)?.find('\n')? + 1;
    }
    let mut units = 0;
    for (index, ch) in text.get(start..)?.char_indices() {
        if units == column {
            return Some(start + index);
        }
        if ch == '\n' || units > column {
            return None;
        }
        units += ch.len_utf16();
    }
    (units == column).then_some(text.len())
}

fn position(text: &str, byte: usize) -> Value {
    let prefix = &text[..byte];
    let line = prefix.bytes().filter(|b| *b == b'\n').count();
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count();
    json!({"line": line, "character": column})
}

fn node_text<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}

/// 只识别接口方法声明名称，不推测调用表达式或普通类方法的归属。
pub fn mapper_method(text: &str, offset: usize) -> Option<(String, String)> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    let mut node = root.descendant_for_byte_range(offset, offset)?;
    while node.kind() != "method_declaration" {
        node = node.parent()?;
    }
    let name = node.child_by_field_name("name")?;
    if !(name.start_byte()..=name.end_byte()).contains(&offset) {
        return None;
    }
    let owner = node.parent()?.parent()?;
    if owner.kind() != "interface_declaration" || owner.parent()?.kind() != "program" {
        return None;
    }
    let interface = node_text(owner.child_by_field_name("name")?, text);
    let mut cursor = root.walk();
    let package = root
        .named_children(&mut cursor)
        .find(|n| n.kind() == "package_declaration")?;
    let mut package_cursor = package.walk();
    let package_name = package
        .named_children(&mut package_cursor)
        .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))?;
    Some((
        format!("{}.{}", node_text(package_name, text), interface),
        node_text(name, text).to_owned(),
    ))
}

fn attribute(event: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)?
        .unescape_value()
        .ok()
        .map(|s| s.into_owned())
}

/// XML 事件解析不会读取或请求外部 DTD；注释和动态 SQL 子节点不作为 SQL 定义。
pub fn xml_locations(text: &str, namespace: &str, method: &str, uri: &str) -> Vec<Value> {
    let mut reader = Reader::from_str(text);
    let mut depth: usize = 0;
    let mut matches = false;
    let mut results = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
                let empty =
                    text.as_bytes().get(reader.buffer_position() as usize - 2) == Some(&b'/');
                if depth == 0 {
                    matches = event.name().as_ref() == b"mapper"
                        && attribute(&event, b"namespace").as_deref() == Some(namespace);
                } else if depth == 1
                    && matches
                    && matches!(
                        event.name().as_ref(),
                        b"select" | b"insert" | b"update" | b"delete"
                    )
                    && attribute(&event, b"id").as_deref() == Some(method)
                {
                    let tag_start = text[start..].find('<').map(|i| start + i).unwrap_or(start);
                    let point = position(text, tag_start);
                    results.push(json!({"uri":uri,"range":{"start":point,"end":point}}));
                }
                if !empty {
                    depth += 1;
                }
            }
            Ok(Event::End(_)) => depth = depth.saturating_sub(1),
            Ok(Event::Eof) => return results,
            Err(_) => return Vec::new(),
            _ => {}
        }
    }
}

pub fn definitions(
    roots: &[std::path::PathBuf],
    namespace: &str,
    method: Option<&str>,
    reverse: bool,
) -> Vec<Value> {
    let mut results = Vec::new();
    for root in roots {
        visit(root, namespace, method, reverse, &mut results);
    }
    results.sort_by_key(|v| v["uri"].as_str().unwrap_or("").to_owned());
    results.dedup();
    results
}

fn visit(
    path: &Path,
    namespace: &str,
    method: Option<&str>,
    reverse: bool,
    results: &mut Vec<Value>,
) {
    let filename = format!("{}.java", namespace.rsplit('.').next().unwrap_or(""));
    visit_sources(path, if reverse { "java" } else { "xml" }, &mut |path| {
        if reverse
            && path
                .file_name()
                .is_none_or(|name| name != filename.as_str())
        {
            return;
        }
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(uri) = Url::from_file_path(path)
        {
            if reverse {
                results.extend(java_locations(&text, namespace, method, uri.as_str()));
            } else if text.contains("<mapper") {
                results.extend(xml_locations(
                    &text,
                    namespace,
                    method.unwrap_or(""),
                    uri.as_str(),
                ));
            }
        }
    });
}

#[derive(Default)]
struct SourceIndex {
    roots: std::collections::HashMap<
        std::path::PathBuf,
        (std::time::Instant, std::sync::Arc<Vec<std::path::PathBuf>>),
    >,
}

fn source_index() -> &'static std::sync::Mutex<SourceIndex> {
    static INDEX: std::sync::OnceLock<std::sync::Mutex<SourceIndex>> = std::sync::OnceLock::new();
    INDEX.get_or_init(Default::default)
}

/// Paths only: no Java AST or project text retained. Opened new files update the index immediately.
pub fn remember_source(path: &Path) {
    if !path
        .extension()
        .is_some_and(|ext| ext == "java" || ext == "xml")
    {
        return;
    }
    let mut index = source_index().lock().unwrap();
    for (root, (_, paths)) in &mut index.roots {
        if path.starts_with(root) && !paths.iter().any(|existing| existing == path) {
            std::sync::Arc::make_mut(paths).push(path.into());
        }
    }
}

pub fn visit_sources(path: &Path, extension: &str, visitor: &mut impl FnMut(&Path)) {
    let cached = {
        let index = source_index().lock().unwrap();
        index
            .roots
            .get(path)
            .filter(|(stamp, _)| stamp.elapsed() < std::time::Duration::from_secs(30))
            .map(|(_, paths)| paths.clone())
    };
    let paths = cached.unwrap_or_else(|| {
        let mut paths = Vec::new();
        collect_sources(path, &mut paths);
        let paths = std::sync::Arc::new(paths);
        let mut index = source_index().lock().unwrap();
        index
            .roots
            .insert(path.into(), (std::time::Instant::now(), paths.clone()));
        paths
    });
    for path in paths
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
    {
        visitor(path);
    }
}

fn collect_sources(path: &Path, paths: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.')
                || matches!(
                    name.as_ref(),
                    "target" | "build" | "node_modules" | "temp" | "bin" | "out"
                )
            {
                continue;
            }
            collect_sources(&path, paths);
        } else if kind.is_file()
            && path.extension().is_some_and(|e| e == "java" || e == "xml")
            && entry.metadata().is_ok_and(|m| m.len() <= 4 * 1024 * 1024)
        {
            paths.push(path);
        }
    }
}

/// 只在 mapper namespace 或顶层 SQL id 的属性值上提供反向跳转。
pub fn xml_target(text: &str, offset: usize) -> Option<(String, Option<String>)> {
    let mut reader = Reader::from_str(text);
    let mut namespace = None;
    let mut depth = 0usize;
    loop {
        let event = reader.read_event().ok()?;
        let (event, empty) = match event {
            Event::Start(event) => (event, false),
            Event::Empty(event) => (event, true),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Eof => return None,
            _ => continue,
        };
        if depth == 0 && event.name().as_ref() == b"mapper" {
            namespace = attribute(&event, b"namespace");
            if attribute_hit(&event, b"namespace", text, offset) {
                return namespace.map(|name| (name, None));
            }
        } else if depth == 1
            && namespace.is_some()
            && matches!(
                event.name().as_ref(),
                b"select" | b"insert" | b"update" | b"delete"
            )
            && attribute_hit(&event, b"id", text, offset)
        {
            return Some((namespace?, Some(attribute(&event, b"id")?)));
        }
        if !empty {
            depth += 1;
        }
    }
}

fn attribute_hit(
    event: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
    text: &str,
    offset: usize,
) -> bool {
    event.attributes().flatten().any(|attribute| {
        if attribute.key.as_ref() != key {
            return false;
        }
        let start = (attribute.value.as_ptr() as usize).checked_sub(text.as_ptr() as usize);
        start.is_some_and(|start| start <= offset && offset < start + attribute.value.len())
    })
}

pub fn java_locations(text: &str, namespace: &str, method: Option<&str>, uri: &str) -> Vec<Value> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut cursor = root.walk();
    let Some(package) = root
        .named_children(&mut cursor)
        .find(|n| n.kind() == "package_declaration")
    else {
        return Vec::new();
    };
    let mut cursor = package.walk();
    let Some(package) = package
        .named_children(&mut cursor)
        .find(|n| matches!(n.kind(), "identifier" | "scoped_identifier"))
    else {
        return Vec::new();
    };
    let mut cursor = root.walk();
    let mut results = Vec::new();
    for interface in root.named_children(&mut cursor).filter(|n| {
        if method.is_some() {
            n.kind() == "interface_declaration"
        } else {
            matches!(
                n.kind(),
                "interface_declaration"
                    | "class_declaration"
                    | "record_declaration"
                    | "enum_declaration"
                    | "annotation_type_declaration"
            )
        }
    }) {
        let Some(name) = interface.child_by_field_name("name") else {
            continue;
        };
        if format!("{}.{}", node_text(package, text), node_text(name, text)) != namespace {
            continue;
        }
        let mut targets = Vec::new();
        if let Some(method) = method {
            if let Some(body) = interface.child_by_field_name("body") {
                let mut cursor = body.walk();
                for declaration in body
                    .named_children(&mut cursor)
                    .filter(|n| n.kind() == "method_declaration")
                {
                    if let Some(name) = declaration.child_by_field_name("name")
                        && node_text(name, text) == method
                    {
                        targets.push(name);
                    }
                }
            }
        } else {
            targets.push(name);
        }
        for name in targets {
            results.push(json!({"uri":uri,"range":{"start":position(text,name.start_byte()),"end":position(text,name.end_byte())}}));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_attribute_navigation_does_not_match_sql_body_or_comments() {
        let xml = "<mapper namespace='demo.StoreMapper'><!-- find --><select id='find'>find</select><sql id='fragment'/></mapper>";
        assert_eq!(
            xml_target(xml, xml.find("demo.StoreMapper").unwrap()),
            Some(("demo.StoreMapper".into(), None))
        );
        assert_eq!(
            xml_target(xml, xml.find("id='find'").unwrap() + 4),
            Some(("demo.StoreMapper".into(), Some("find".into())))
        );
        assert!(xml_target(xml, xml.find("<!-- find").unwrap() + 5).is_none());
        assert!(xml_target(xml, xml.find(">find<").unwrap() + 1).is_none());
        assert!(xml_target(xml, xml.find("fragment").unwrap()).is_none());
    }

    #[test]
    fn reverse_navigation_requires_exact_java_namespace_and_method() {
        let java = "package demo; public interface StoreMapper {\n String find();\n String find(int id);\n}";
        let results = java_locations(
            java,
            "demo.StoreMapper",
            Some("find"),
            "file:///StoreMapper.java",
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["range"]["start"]["line"], 1);
        assert_eq!(
            java_locations(java, "demo.StoreMapper", None, "file:///x").len(),
            1
        );
        assert!(java_locations(java, "wrong.StoreMapper", Some("find"), "file:///x").is_empty());
        assert!(java_locations(java, "demo.StoreMapper", Some("missing"), "file:///x").is_empty());
    }

    #[test]
    fn only_interface_method_name_is_accepted() {
        let source = "package demo; public interface StoreMapper { String find(int id); }";
        assert_eq!(
            mapper_method(source, source.find("find").unwrap()),
            Some(("demo.StoreMapper".into(), "find".into()))
        );
        assert_eq!(
            mapper_method(source, source.find("find").unwrap() + "find".len()),
            Some(("demo.StoreMapper".into(), "find".into()))
        );
        assert!(mapper_method(source, source.find("String").unwrap()).is_none());
        let class = source.replace("interface", "class");
        assert!(mapper_method(&class, class.find("find").unwrap()).is_none());
        assert!(mapper_method("package demo; class X { void x(){ dao.find(); } }", 43).is_none());
    }

    #[test]
    fn namespace_and_statement_id_must_both_match() {
        let xml = "<!DOCTYPE mapper SYSTEM 'https://invalid.example/no-fetch'><mapper namespace='demo.StoreMapper'>\n<!-- <select id='find'/> -->\n<select id='find'>中文<select id='nested'/></select><sql id='find'/><delete id='remove'/></mapper>";
        let locations = xml_locations(xml, "demo.StoreMapper", "find", "file:///mapper.xml");
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0]["range"]["start"]["line"], 2);
        assert!(xml_locations(xml, "other.StoreMapper", "find", "file:///x").is_empty());
        assert!(xml_locations(xml, "demo.StoreMapper", "nested", "file:///x").is_empty());
        assert_eq!(
            xml_locations(xml, "demo.StoreMapper", "remove", "file:///x").len(),
            1
        );
    }

    #[test]
    fn utf16_position_handles_chinese_and_surrogate_pairs() {
        let text = "第一行\r\n😀方法 find";
        assert_eq!(byte_offset(text, 1, 5), text.find("find"));
        assert!(byte_offset(text, 1, 1).is_none());
        assert!(byte_offset(text, 30, 0).is_none());
    }
}

/// Resolve declared fields (including Lombok fields) and record components, never local variables.
pub fn field_locations(text: &str, owner: &str, field: &str, uri: &str) -> Vec<Value> {
    if java_locations(text, owner, None, uri).is_empty() {
        return Vec::new();
    }
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    let mut cursor = tree.root_node().walk();
    let mut results = Vec::new();
    for class in tree.root_node().named_children(&mut cursor) {
        if class
            .child_by_field_name("name")
            .is_none_or(|name| node_text(name, text) != owner.rsplit('.').next().unwrap_or(""))
        {
            continue;
        }
        for container in [
            class.child_by_field_name("body"),
            class.child_by_field_name("parameters"),
        ]
        .into_iter()
        .flatten()
        {
            let mut cursor = container.walk();
            for declaration in container.named_children(&mut cursor) {
                let mut names = Vec::new();
                if declaration.kind() == "field_declaration" {
                    let mut cursor = declaration.walk();
                    for variable in declaration
                        .named_children(&mut cursor)
                        .filter(|node| node.kind() == "variable_declarator")
                    {
                        if let Some(name) = variable.child_by_field_name("name") {
                            names.push(name);
                        }
                    }
                } else if declaration.kind() == "formal_parameter"
                    && let Some(name) = declaration.child_by_field_name("name")
                {
                    names.push(name);
                }
                for name in names {
                    if node_text(name, text) == field {
                        results.push(json!({"uri":uri,"range":{"start":position(text,name.start_byte()),"end":position(text,name.end_byte())}}));
                    }
                }
            }
        }
    }
    results
}
