use anyhow::{Context as _, Result, bail, ensure};
use quick_xml::{Reader, events::Event};
use std::collections::{BTreeMap, BTreeSet};

pub const PHASES: &[&str] = &[
    "clean", "validate", "compile", "test", "package", "verify", "install", "site", "deploy",
];

#[derive(Default, Debug)]
struct Element {
    name: String,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn child(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|child| child.name == name)
    }
    fn value(&self, name: &str) -> String {
        self.child(name)
            .map(|child| child.text.trim().to_owned())
            .unwrap_or_default()
    }
    fn items(&self, container: &str, name: &str) -> Vec<&Self> {
        self.child(container)
            .map(|node| {
                node.children
                    .iter()
                    .filter(|child| child.name == name)
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn xml(source: &str) -> Result<Element> {
    ensure!(source.len() <= 2 * 1024 * 1024, "POM 超过 2 MiB，已跳过");
    let mut reader = Reader::from_str(source);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = vec![Element::default()];
    loop {
        match reader.read_event().context("无法解析 POM XML")? {
            Event::Start(tag) => {
                let element = Element {
                    name: String::from_utf8_lossy(tag.local_name().as_ref()).into_owned(),
                    ..Default::default()
                };
                stack.push(element);
            }
            Event::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text.decode()?);
                }
            }
            Event::CData(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(reference) => {
                let reference = reference.decode()?;
                let character = match reference.as_ref() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    value if value.starts_with("#x") => {
                        char::from_u32(u32::from_str_radix(&value[2..], 16)?)
                            .context("无效 XML 字符")?
                    }
                    value if value.starts_with('#') => {
                        char::from_u32(value[1..].parse()?).context("无效 XML 字符")?
                    }
                    _ => bail!("不支持 POM 外部实体"),
                };
                if let Some(node) = stack.last_mut() {
                    node.text.push(character);
                }
            }
            Event::End(_) => {
                ensure!(stack.len() > 1, "POM 标签不完整");
                if let Some(node) = stack.pop()
                    && let Some(parent) = stack.last_mut()
                {
                    parent.children.push(node);
                }
            }
            Event::DocType(_) => bail!("POM 不允许外部 DTD"),
            Event::Eof => break,
            _ => {}
        }
        ensure!(stack.len() <= 128, "POM 嵌套过深");
    }
    ensure!(stack.len() == 1, "POM 标签未闭合");
    stack
        .pop()
        .and_then(|root| {
            root.children
                .into_iter()
                .find(|node| node.name == "project")
        })
        .context("不是 Maven POM")
}

#[derive(Clone, Debug)]
pub struct MavenPlugin {
    pub name: String,
    pub coordinate: String,
    pub goals: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MavenProfile {
    pub id: String,
    pub active_by_default: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Pom {
    pub name: String,
    pub coordinate: String,
    pub packaging: String,
    pub modules: Vec<String>,
    pub plugins: Vec<MavenPlugin>,
    pub profiles: Vec<MavenProfile>,
    pub dependencies: Vec<String>,
}

fn interpolate(value: &str, properties: &BTreeMap<String, String>) -> String {
    let mut result = value.to_owned();
    for _ in 0..8 {
        let previous = result.clone();
        for (key, value) in properties {
            result = result.replace(&format!("${{{key}}}"), value);
        }
        if result == previous {
            break;
        }
    }
    result
}

fn known_goals(artifact: &str) -> &'static [&'static str] {
    match artifact {
        "maven-clean-plugin" => &["clean"],
        "maven-compiler-plugin" => &["compile", "testCompile"],
        "maven-resources-plugin" => &["resources", "testResources"],
        "maven-surefire-plugin" => &["test"],
        "maven-failsafe-plugin" => &["integration-test", "verify"],
        "maven-jar-plugin" => &["jar", "test-jar"],
        "maven-war-plugin" => &["war"],
        "maven-install-plugin" => &["install"],
        "maven-deploy-plugin" => &["deploy"],
        "maven-dependency-plugin" => &["tree", "analyze", "resolve", "sources"],
        "maven-source-plugin" => &["jar-no-fork"],
        "maven-javadoc-plugin" => &["javadoc", "jar"],
        "maven-help-plugin" => &["effective-pom", "active-profiles", "describe"],
        "spring-boot-maven-plugin" => &["run", "repackage", "start", "stop", "build-image"],
        "exec-maven-plugin" => &["exec", "java"],
        _ => &[],
    }
}

pub fn parse_pom(source: &str) -> Result<Pom> {
    let root = xml(source)?;
    let mut properties: BTreeMap<String, String> = root
        .child("properties")
        .map(|node| {
            node.children
                .iter()
                .map(|child| (child.name.clone(), child.text.trim().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    for key in ["groupId", "version"] {
        let value = root
            .child(key)
            .map(|node| node.text.trim().to_owned())
            .unwrap_or_else(|| {
                root.child("parent")
                    .map(|parent| parent.value(key))
                    .unwrap_or_default()
            });
        properties.insert(format!("project.{key}"), value);
    }
    properties.insert("project.artifactId".into(), root.value("artifactId"));
    let artifact = interpolate(&root.value("artifactId"), &properties);
    ensure!(!artifact.is_empty(), "POM 缺少 artifactId");
    let name = interpolate(&root.value("name"), &properties);
    let mut plugins = Vec::new();
    if let Some(build) = root.child("build") {
        for plugin in build.items("plugins", "plugin") {
            let artifact = plugin.value("artifactId");
            if artifact.is_empty() {
                continue;
            }
            let group = plugin.value("groupId");
            let group = if group.is_empty() {
                "org.apache.maven.plugins".into()
            } else {
                group
            };
            let version = interpolate(&plugin.value("version"), &properties);
            let coordinate = if version.is_empty() || version.contains("${") {
                format!("{group}:{artifact}")
            } else {
                format!("{group}:{artifact}:{version}")
            };
            let mut goals: BTreeSet<String> = known_goals(&artifact)
                .iter()
                .map(|goal| (*goal).to_owned())
                .collect();
            for execution in plugin.items("executions", "execution") {
                for goal in execution.items("goals", "goal") {
                    goals.insert(goal.text.trim().to_owned());
                }
            }
            plugins.push(MavenPlugin {
                name: artifact,
                coordinate,
                goals: goals.into_iter().filter(|goal| !goal.is_empty()).collect(),
            });
        }
    }
    for artifact in [
        "maven-clean-plugin",
        "maven-compiler-plugin",
        "maven-resources-plugin",
        "maven-surefire-plugin",
        "maven-install-plugin",
        "maven-deploy-plugin",
        "maven-dependency-plugin",
        "maven-help-plugin",
    ] {
        if !plugins.iter().any(|plugin| plugin.name == artifact) {
            plugins.push(MavenPlugin {
                name: artifact.into(),
                coordinate: format!("org.apache.maven.plugins:{artifact}"),
                goals: known_goals(artifact)
                    .iter()
                    .map(|goal| (*goal).into())
                    .collect(),
            });
        }
    }
    plugins.sort_by(|left, right| left.name.cmp(&right.name));
    let dependencies = root
        .items("dependencies", "dependency")
        .iter()
        .map(|dependency| {
            let coordinate = format!(
                "{}:{}:{}",
                dependency.value("groupId"),
                dependency.value("artifactId"),
                dependency.value("version")
            );
            let scope = dependency.value("scope");
            let coordinate = interpolate(&coordinate, &properties);
            if scope.is_empty() {
                coordinate
            } else {
                format!("{coordinate} [{scope}]")
            }
        })
        .collect();
    let profiles = root
        .items("profiles", "profile")
        .iter()
        .map(|profile| MavenProfile {
            id: profile.value("id"),
            active_by_default: profile
                .child("activation")
                .is_some_and(|activation| activation.value("activeByDefault") == "true"),
        })
        .filter(|profile| !profile.id.is_empty())
        .collect();
    Ok(Pom {
        name: if name.is_empty() || name.contains("${") {
            artifact.clone()
        } else {
            name
        },
        coordinate: format!(
            "{}:{artifact}:{}",
            interpolate("${project.groupId}", &properties),
            interpolate("${project.version}", &properties)
        ),
        packaging: if root.value("packaging").is_empty() {
            "jar".into()
        } else {
            root.value("packaging")
        },
        modules: root
            .items("modules", "module")
            .iter()
            .map(|module| interpolate(module.text.trim(), &properties))
            .collect(),
        plugins,
        profiles,
        dependencies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_namespaces_profiles_plugins_and_coordinates() {
        let pom = parse_pom(r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><parent><groupId>com.example</groupId><version>1.0</version></parent><artifactId>service</artifactId><name>Service &amp; API</name><properties><boot.version>3.4.1</boot.version></properties><modules><module>core</module></modules><profiles><profile><id>dev</id><activation><activeByDefault>true</activeByDefault></activation></profile></profiles><build><plugins><plugin><groupId>org.springframework.boot</groupId><artifactId>spring-boot-maven-plugin</artifactId><version>${boot.version}</version><executions><execution><goals><goal>custom</goal></goals></execution></executions></plugin></plugins></build><dependencies><dependency><groupId>org.example</groupId><artifactId>api</artifactId><version>${project.version}</version><scope>test</scope></dependency></dependencies></project>"#).expect("valid POM");
        assert_eq!(pom.coordinate, "com.example:service:1.0");
        assert_eq!(pom.name, "Service & API");
        assert_eq!(pom.modules, ["core"]);
        assert!(pom.profiles[0].active_by_default);
        let boot = pom
            .plugins
            .iter()
            .find(|plugin| plugin.name == "spring-boot-maven-plugin")
            .expect("Spring Boot plugin");
        assert!(boot.goals.contains(&"run".into()));
        assert!(boot.goals.contains(&"custom".into()));
        assert_eq!(
            boot.coordinate,
            "org.springframework.boot:spring-boot-maven-plugin:3.4.1"
        );
        assert_eq!(pom.dependencies, ["org.example:api:1.0 [test]"]);
    }
    #[test]
    fn rejects_external_entities_and_malformed_poms() {
        assert!(parse_pom("<!DOCTYPE project SYSTEM 'file:///private/data'><project/>").is_err());
        assert!(parse_pom("<project><artifactId>broken</project>").is_err());
    }
}
