pub use crate::pom::PHASES;
use crate::pom::{Pom, parse_pom};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use worktree::WorktreeId;

pub struct ScanRoot {
    pub id: WorktreeId,
    pub path: PathBuf,
    pub poms: Vec<PathBuf>,
}

#[derive(Clone)]
pub struct MavenModule {
    pub worktree: WorktreeId,
    pub root: PathBuf,
    pub path: PathBuf,
    pub reactor: PathBuf,
    pub pom: Pom,
    pub children: Vec<usize>,
}

#[derive(Default)]
pub struct MavenModel {
    pub modules: Vec<MavenModule>,
    pub roots: Vec<usize>,
    pub warnings: Vec<String>,
}

pub fn scan(roots: Vec<ScanRoot>) -> MavenModel {
    let mut model = MavenModel::default();
    for root in roots {
        let Ok(canonical_root) = root.path.canonicalize() else {
            continue;
        };
        for path in root.poms.into_iter().take(512) {
            let path = root.path.join(path);
            let result = (|| -> Result<Pom> {
                ensure!(
                    path.canonicalize()?.starts_with(&canonical_root),
                    "POM 位于工作区外"
                );
                ensure!(
                    std::fs::metadata(&path)?.len() <= 2 * 1024 * 1024,
                    "POM 文件过大"
                );
                parse_pom(&std::fs::read_to_string(&path)?)
            })();
            match result {
                Ok(pom) => model.modules.push(MavenModule {
                    worktree: root.id,
                    root: root.path.clone(),
                    reactor: path.clone(),
                    path,
                    pom,
                    children: Vec::new(),
                }),
                Err(error) => model
                    .warnings
                    .push(format!("{}: {error:#}", path.display())),
            }
        }
    }
    let paths: BTreeMap<_, _> = model
        .modules
        .iter()
        .enumerate()
        .filter_map(|(index, module)| module.path.canonicalize().ok().map(|path| (path, index)))
        .collect();
    let mut children = BTreeSet::new();
    for index in 0..model.modules.len() {
        let module = &model.modules[index];
        let Some(directory) = module.path.parent() else {
            continue;
        };
        let mut linked = Vec::new();
        for relative in &module.pom.modules {
            let path = directory.join(relative);
            let path = if path.file_name().is_some_and(|name| name == "pom.xml") {
                path
            } else {
                path.join("pom.xml")
            };
            if let Some(child) = path
                .canonicalize()
                .ok()
                .and_then(|path| paths.get(&path).copied())
            {
                if child != index {
                    linked.push(child);
                    children.insert(child);
                }
            } else {
                model
                    .warnings
                    .push(format!("模块未加载或已被忽略：{}", path.display()));
            }
        }
        model.modules[index].children = linked;
    }
    model.roots = (0..model.modules.len())
        .filter(|index| !children.contains(index))
        .collect();
    let mut visited = BTreeSet::new();
    for root in model.roots.clone() {
        assign_reactor(&mut model.modules, root, None, &mut visited);
    }
    for index in 0..model.modules.len() {
        if !visited.contains(&index) {
            model.roots.push(index);
            assign_reactor(&mut model.modules, index, None, &mut visited);
        }
    }
    model
}

fn assign_reactor(
    modules: &mut [MavenModule],
    index: usize,
    reactor: Option<PathBuf>,
    visited: &mut BTreeSet<usize>,
) {
    if !visited.insert(index) {
        return;
    }
    let Some(module) = modules.get_mut(index) else {
        return;
    };
    let reactor = reactor.unwrap_or_else(|| module.path.clone());
    module.reactor = reactor.clone();
    for child in module.children.clone() {
        assign_reactor(modules, child, Some(reactor.clone()), visited);
    }
}
