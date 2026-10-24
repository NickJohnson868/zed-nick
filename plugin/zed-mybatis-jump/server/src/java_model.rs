use crate::navigation;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
use tree_sitter::{Node, Parser};

#[derive(Clone)]
pub struct Parameter {
    pub name: String,
    pub alias: Option<String>,
    pub kind: String,
}
#[derive(Default)]
pub struct Model {
    pub name: String,
    pub package: String,
    pub imports: Vec<String>,
    pub fields: BTreeMap<String, String>,
    pub methods: HashMap<String, Vec<Vec<Parameter>>>,
    pub parent: Option<String>,
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn parsed(source: &str) -> Option<Model> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut model = Model::default();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "package_declaration" => {
                model.package = text(node, source)
                    .trim_start_matches("package")
                    .trim()
                    .trim_end_matches(';')
                    .into()
            }
            "import_declaration" => {
                let value = text(node, source)
                    .trim_start_matches("import")
                    .trim()
                    .trim_end_matches(';');
                if !value.starts_with("static ") {
                    model.imports.push(value.into());
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "record_declaration"
            | "enum_declaration" => {
                let name = text(node.child_by_field_name("name")?, source);
                model.name = format!("{}.{}", model.package, name);
                model.parent = node.child_by_field_name("superclass").map(|n| {
                    text(n, source)
                        .trim_start_matches("extends")
                        .trim()
                        .to_owned()
                });
                if let Some(params) = node.child_by_field_name("parameters") {
                    read_fields(params, source, &mut model);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    read_fields(body, source, &mut model);
                }
            }
            _ => {}
        }
    }
    (!model.name.is_empty()).then_some(model)
}
fn read_fields(body: Node<'_>, source: &str, model: &mut Model) {
    let mut cursor = body.walk();
    for node in body.named_children(&mut cursor) {
        match node.kind() {
            "field_declaration" => {
                let Some(kind) = node.child_by_field_name("type") else {
                    continue;
                };
                let mut cursor = node.walk();
                for variable in node
                    .named_children(&mut cursor)
                    .filter(|n| n.kind() == "variable_declarator")
                {
                    if let Some(name) = variable.child_by_field_name("name") {
                        model
                            .fields
                            .insert(text(name, source).into(), text(kind, source).into());
                    }
                }
            }
            "formal_parameter" => {
                if let (Some(name), Some(kind)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("type"),
                ) {
                    model
                        .fields
                        .insert(text(name, source).into(), text(kind, source).into());
                }
            }
            "method_declaration" => {
                let Some(name) = node.child_by_field_name("name") else {
                    continue;
                };
                let name = text(name, source);
                let Some(params) = node.child_by_field_name("parameters") else {
                    continue;
                };
                let mut parameters = Vec::new();
                let mut cursor = params.walk();
                for parameter in params
                    .named_children(&mut cursor)
                    .filter(|n| matches!(n.kind(), "formal_parameter" | "spread_parameter"))
                {
                    let (Some(name), Some(kind)) = (
                        parameter.child_by_field_name("name"),
                        parameter.child_by_field_name("type"),
                    ) else {
                        continue;
                    };
                    let mut alias = None;
                    let mut pc = parameter.walk();
                    for modifiers in parameter
                        .named_children(&mut pc)
                        .filter(|n| n.kind() == "modifiers")
                    {
                        let mut mc = modifiers.walk();
                        for annotation in modifiers.named_children(&mut mc) {
                            if annotation.child_by_field_name("name").is_some_and(|n| {
                                text(n, source).rsplit('.').next() == Some("Param")
                            }) {
                                let raw = text(annotation, source);
                                alias = raw.split('"').nth(1).map(str::to_owned);
                            }
                        }
                    }
                    parameters.push(Parameter {
                        name: text(name, source).into(),
                        alias,
                        kind: text(kind, source).into(),
                    });
                }
                // Public bean accessors may expose a property without a field of the same name.
                let bean = if parameters.is_empty() {
                    name.strip_prefix("get").or_else(|| name.strip_prefix("is"))
                } else if parameters.len() == 1 {
                    name.strip_prefix("set")
                } else {
                    None
                };
                if let Some(suffix) = bean
                    && suffix.chars().next().is_some_and(|c| c.is_uppercase())
                {
                    let property = if suffix.chars().take(2).all(|c| c.is_uppercase()) {
                        suffix.into()
                    } else {
                        {
                            let first = suffix.chars().next().unwrap();
                            format!("{}{}", first.to_lowercase(), &suffix[first.len_utf8()..])
                        }
                    };
                    let kind = if parameters.is_empty() {
                        node.child_by_field_name("type")
                            .map(|n| text(n, source))
                            .unwrap_or("")
                            .into()
                    } else {
                        parameters[0].kind.clone()
                    };
                    model.fields.entry(property).or_insert(kind);
                }
                model
                    .methods
                    .entry(name.into())
                    .or_default()
                    .push(parameters);
            }
            _ => {}
        }
    }
}

type Cache = HashMap<PathBuf, (Option<SystemTime>, Arc<Model>)>;
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
pub fn load(roots: &[PathBuf], name: &str) -> Option<Arc<Model>> {
    let mut result = None;
    for root in roots {
        navigation::visit_sources(root, "java", &mut |path| {
            if path
                .file_stem()
                .is_none_or(|s| s != name.rsplit('.').next().unwrap_or(""))
            {
                return;
            }
            if let Some(model) = file(path)
                && model.name == name
            {
                result = Some(model);
            }
        });
    }
    result
}
fn file(path: &Path) -> Option<Arc<Model>> {
    let stamp = fs::metadata(path).ok()?.modified().ok();
    if let Some((previous, model)) = cache().lock().ok()?.get(path)
        && *previous == stamp
    {
        return Some(model.clone());
    }
    let source = fs::read_to_string(path).ok()?;
    if source.len() > 4 * 1024 * 1024 {
        return None;
    }
    let model = Arc::new(parsed(&source)?);
    let mut cache = cache().lock().ok()?;
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(path.into(), (stamp, model.clone()));
    Some(model)
}
pub fn resolve(model: &Model, kind: &str) -> Option<String> {
    if kind.contains(['<', '[', '?']) {
        return None;
    }
    if kind.contains('.') {
        return Some(kind.into());
    }
    if let Some(import) = model
        .imports
        .iter()
        .find(|i| i.rsplit('.').next() == Some(kind))
    {
        return Some(import.clone());
    }
    Some(format!("{}.{}", model.package, kind))
}
/// None means the type could not be established; never turn missing metadata into an error.
pub fn fields(roots: &[PathBuf], name: &str) -> Option<(BTreeMap<String, String>, bool)> {
    fields_at(roots, name, 0)
}
fn fields_at(
    roots: &[PathBuf],
    name: &str,
    depth: usize,
) -> Option<(BTreeMap<String, String>, bool)> {
    if depth > 8 {
        return None;
    }
    let model = load(roots, name)?;
    let mut fields = model
        .fields
        .iter()
        .map(|(name, kind)| (name.clone(), resolve(&model, kind).unwrap_or_default()))
        .collect::<BTreeMap<_, _>>();
    let mut complete = true;
    if let Some(parent) = &model.parent
        && parent != "Object"
        && parent != "java.lang.Object"
    {
        if let Some((inherited, known)) =
            resolve(&model, parent).and_then(|p| fields_at(roots, &p, depth + 1))
        {
            for (k, v) in inherited {
                fields.entry(k).or_insert(v);
            }
            complete = known;
        } else {
            complete = false;
        }
    }
    Some((fields, complete))
}
pub fn types(roots: &[PathBuf], prefix: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for root in roots {
        navigation::visit_sources(root, "java", &mut |path| {
            if candidates.len() < 100
                && path.file_stem().is_some_and(|n| {
                    n.to_string_lossy()
                        .to_lowercase()
                        .contains(&prefix.rsplit('.').next().unwrap_or("").to_lowercase())
                })
            {
                candidates.push(path.to_owned());
            }
        });
    }
    candidates
        .iter()
        .filter_map(|p| file(p))
        .map(|m| m.name.clone())
        .filter(|name| name.to_lowercase().contains(&prefix.to_lowercase()))
        .take(100)
        .collect()
}
