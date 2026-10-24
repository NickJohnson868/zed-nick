use crate::navigation;
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use url::Url;

#[derive(Debug, PartialEq)]
pub enum Target {
    Java(String),
    Field {
        owner: String,
        name: String,
    },
    Reference {
        namespace: String,
        id: String,
        tag: &'static str,
    },
}

fn attr(event: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)?
        .unescape_value()
        .ok()
        .map(|v| v.into_owned())
}

pub fn target(text: &str, offset: usize) -> Option<Target> {
    let mut reader = Reader::from_str(text);
    let mut namespace = None;
    let mut depth = 0usize;
    let mut owners: Vec<Option<String>> = Vec::new();
    loop {
        let (event, empty) = match reader.read_event().ok()? {
            Event::Start(event) => (event, false),
            Event::Empty(event) => (event, true),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                owners.pop();
                continue;
            }
            Event::Eof => return None,
            _ => continue,
        };
        if depth == 0 {
            if event.name().as_ref() != b"mapper" {
                return None;
            }
            namespace = attr(&event, b"namespace");
        } else if let Some(namespace) = &namespace {
            for attribute in event.attributes().flatten() {
                let start =
                    (attribute.value.as_ptr() as usize).checked_sub(text.as_ptr() as usize)?;
                if !(start..=start + attribute.value.len()).contains(&offset) {
                    continue;
                }
                let raw = std::str::from_utf8(&attribute.value).ok()?;
                let key = attribute.key.as_ref();
                let tag = match key {
                    b"resultMap" => Some("resultMap"),
                    b"extends" if event.name().as_ref() == b"resultMap" => Some("resultMap"),
                    b"refid" if event.name().as_ref() == b"include" => Some("sql"),
                    _ => None,
                };
                if let Some(tag) = tag {
                    // Multiple result maps are comma separated; resolve only the value under the caret.
                    let relative = offset - start;
                    let mut cursor = 0;
                    for part in raw.split(',') {
                        let end = cursor + part.len();
                        if (cursor..=end).contains(&relative) {
                            let value = quick_xml::escape::unescape(part.trim()).ok()?;
                            if value.is_empty() || value.contains("${") {
                                return None;
                            }
                            let (owner, id) = value.rsplit_once('.').unwrap_or((namespace, &value));
                            return Some(Target::Reference {
                                namespace: owner.into(),
                                id: id.into(),
                                tag,
                            });
                        }
                        cursor = end + 1;
                    }
                } else if key == b"property" {
                    let owner = owners.last()?.as_ref()?.clone();
                    let name = attribute.unescape_value().ok()?.into_owned();
                    if !name.contains('.') && !name.is_empty() {
                        return Some(Target::Field { owner, name });
                    }
                } else if matches!(
                    key,
                    b"type" | b"resultType" | b"parameterType" | b"javaType" | b"ofType"
                ) {
                    let value = attribute.unescape_value().ok()?;
                    // Exact project Java names only: built-in MyBatis aliases are not guessed.
                    if value.contains('.') && !value.contains("${") {
                        return Some(Target::Java(value.into_owned()));
                    }
                }
            }
        }
        if !empty {
            let owner = match event.name().as_ref() {
                b"resultMap" => attr(&event, b"type"),
                b"association" => attr(&event, b"javaType"),
                b"collection" => attr(&event, b"ofType"),
                _ => owners.last().cloned().flatten(),
            };
            owners.push(owner);
            depth += 1;
        }
    }
}

pub fn definitions(roots: &[PathBuf], text: &str, offset: usize, uri: &str) -> Option<Vec<Value>> {
    let target = target(text, offset)?;
    match target {
        Target::Java(name) => Some(navigation::definitions(roots, &name, None, true)),
        Target::Field { owner, name } => {
            let mut results = Vec::new();
            for root in roots {
                navigation::visit_sources(root, "java", &mut |path| {
                    if path.file_name().is_none_or(|file| {
                        file != format!("{}.java", owner.rsplit('.').next().unwrap_or("")).as_str()
                    }) {
                        return;
                    }
                    if let Ok(source) = fs::read_to_string(path)
                        && let Ok(uri) = Url::from_file_path(path)
                    {
                        results.extend(navigation::field_locations(
                            &source,
                            &owner,
                            &name,
                            uri.as_str(),
                        ));
                    }
                });
            }
            Some(results)
        }
        Target::Reference { namespace, id, tag } => {
            let mut results = locations(text, &namespace, &id, tag, uri);
            if !results.is_empty() {
                return Some(results);
            }
            // Prefer the live source buffer for local definitions, including unsaved changes.
            for root in roots {
                navigation::visit_sources(root, "xml", &mut |path| {
                    let Ok(candidate) = Url::from_file_path(path) else {
                        return;
                    };
                    if candidate.as_str() == uri {
                        return;
                    }
                    if let Ok(source) = fs::read_to_string(path)
                        && source.contains("<mapper")
                    {
                        results.extend(locations(
                            &source,
                            &namespace,
                            &id,
                            tag,
                            candidate.as_str(),
                        ));
                    }
                });
            }
            results.sort_by_key(|value| value["uri"].as_str().unwrap_or("").to_owned());
            results.dedup();
            Some(results)
        }
    }
}

fn locations(text: &str, namespace: &str, id: &str, tag: &str, uri: &str) -> Vec<Value> {
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut owner = false;
    let mut results = Vec::new();
    loop {
        let (event, empty) = match reader.read_event() {
            Ok(Event::Start(event)) => (event, false),
            Ok(Event::Empty(event)) => (event, true),
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
                continue;
            }
            Ok(Event::Eof) => return results,
            Err(_) => return Vec::new(),
            _ => continue,
        };
        if depth == 0 {
            owner = event.name().as_ref() == b"mapper"
                && attr(&event, b"namespace").as_deref() == Some(namespace);
        }
        if depth == 1
            && owner
            && event.name().as_ref() == tag.as_bytes()
            && attr(&event, b"id").as_deref() == Some(id)
            && let Some(attribute) = event
                .attributes()
                .flatten()
                .find(|a| a.key.as_ref() == b"id")
        {
            let start = attribute.value.as_ptr() as usize - text.as_ptr() as usize;
            results.push(json!({"uri":uri,"range":{"start":crate::sql_text::position(text,start),"end":crate::sql_text::position(text,start+attribute.value.len())}}));
        }
        if !empty {
            depth += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_scope_lists_and_type_attributes() {
        let text = "<mapper namespace='demo.M'><resultMap id='base' type='demo.Model'/><resultMap id='child' extends='base'/><sql id='cols'/><select id='x' resultMap='base, other.N.map'><include refid='cols'/></select></mapper>";
        assert_eq!(
            target(text, text.find("demo.Model").unwrap()),
            Some(Target::Java("demo.Model".into()))
        );
        assert_eq!(
            target(text, text.find("other.N.map").unwrap()),
            Some(Target::Reference {
                namespace: "other.N".into(),
                id: "map".into(),
                tag: "resultMap"
            })
        );
        assert_eq!(
            target(text, text.find("refid='cols'").unwrap() + 7),
            Some(Target::Reference {
                namespace: "demo.M".into(),
                id: "cols".into(),
                tag: "sql"
            })
        );
        assert_eq!(
            locations(text, "demo.M", "base", "resultMap", "file:///x").len(),
            1
        );
        assert!(locations(text, "other.M", "cols", "sql", "file:///x").is_empty());
        assert!(target("<project type='demo.Model'/>", 20).is_none());
    }
}

/// Explicit source range prevents XML attribute strings from being treated as a whole symbol.
pub fn origin_range(text: &str, offset: usize) -> Option<Value> {
    let mut reader = Reader::from_str(text);
    loop {
        let event = match reader.read_event().ok()? {
            Event::Start(event) | Event::Empty(event) => event,
            Event::Eof => return None,
            _ => continue,
        };
        for attribute in event.attributes().flatten() {
            let start = (attribute.value.as_ptr() as usize).checked_sub(text.as_ptr() as usize)?;
            if !(start..=start + attribute.value.len()).contains(&offset) {
                continue;
            }
            let value = std::str::from_utf8(&attribute.value).ok()?;
            let mut cursor = start;
            for part in value.split(',') {
                if (cursor..=cursor + part.len()).contains(&offset) {
                    let begin = cursor + part.len() - part.trim_start().len();
                    let end = cursor + part.trim_end().len();
                    return Some(
                        json!({"start":crate::sql_text::position(text,begin),"end":crate::sql_text::position(text,end)}),
                    );
                }
                cursor += part.len() + 1;
            }
        }
    }
}
