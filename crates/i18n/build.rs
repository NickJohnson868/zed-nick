use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionText {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

fn read_map<T: DeserializeOwned>(path: &Path) -> Result<BTreeMap<String, T>, Box<dyn Error>> {
    println!("cargo:rerun-if-changed={}", path.display());
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn read_maps<T: DeserializeOwned>(directory: &Path) -> Result<BTreeMap<String, T>, Box<dyn Error>> {
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut paths = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut merged = BTreeMap::new();
    for path in paths {
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        for (key, value) in read_map(&path)? {
            if merged.insert(key.clone(), value).is_some() {
                return Err(
                    format!("Duplicate translation key {key:?} in {}", path.display()).into(),
                );
            }
        }
    }
    Ok(merged)
}

fn main() -> Result<(), Box<dyn Error>> {
    let directory =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).join("../../assets/locales/zh-CN");
    let strings: BTreeMap<String, String> = read_maps(&directory.join("strings"))?;
    let actions: BTreeMap<String, ActionText> = read_maps(&directory.join("actions"))?;
    let namespaces: BTreeMap<String, String> = read_map(&directory.join("action_namespaces.json"))?;
    let descriptions: BTreeMap<String, String> = read_maps(&directory.join("schema"))?;
    let mut output = String::from("static SIMPLIFIED_CHINESE: Catalog = Catalog { strings: &[\n");
    for (key, value) in strings {
        writeln!(output, "({key:?}, {value:?}),")?;
    }
    output.push_str("], actions: &[\n");
    for (key, value) in actions {
        writeln!(
            output,
            "({key:?}, {:?}, {:?}),",
            value.name, value.description
        )?;
    }
    output.push_str("], action_namespaces: &[\n");
    for (key, value) in namespaces {
        writeln!(output, "({key:?}, {value:?}),")?;
    }
    output.push_str("], schema_descriptions: &[\n");
    for (key, value) in descriptions {
        writeln!(output, "({key:?}, {value:?}),")?;
    }
    output.push_str("] };\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("catalogs.rs"),
        output,
    )?;
    Ok(())
}
