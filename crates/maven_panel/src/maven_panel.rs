mod model;
mod pom;

use anyhow::{Context as _, Result, ensure};
use db::kvp::KeyValueStore;
use editor::{Editor, EditorEvent};
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Pixels, Render, Subscription, Task, WeakEntity, Window, actions, div, px,
};
use model::{MavenModel, PHASES, ScanRoot};
use project::{Project, TaskSourceKind, trusted_worktrees::TrustedWorktrees};
use serde::{Deserialize, Serialize};
use settings::{DockSide, RegisterSetting, Settings, SettingsStore};
use std::{collections::BTreeSet, path::PathBuf};
use task::{TaskContext, TaskTemplate};
use ui::{Icon, IconButton, IconName, ListItem, Tooltip, prelude::*};
use util::ResultExt as _;
use workspace::{
    Panel, Workspace,
    dock::{DockPosition, PanelEvent},
};

actions!(
    maven_panel,
    [
        ToggleFocus,
        RunSelected,
        SelectNext,
        SelectPrevious,
        Expand,
        Collapse
    ]
);

const TASK_TAG: &str = "maven-panel";

#[derive(Clone, Debug, RegisterSetting)]
struct MavenPanelSettings {
    button: bool,
    starts_open: bool,
    dock: DockSide,
    width: Pixels,
}
impl Settings for MavenPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let panel = content.maven_panel.as_ref();
        Self {
            button: panel.and_then(|panel| panel.button).unwrap_or(true),
            starts_open: panel.and_then(|panel| panel.starts_open).unwrap_or(false),
            dock: panel
                .and_then(|panel| panel.dock)
                .unwrap_or(DockSide::Right),
            width: px(panel
                .and_then(|panel| panel.default_width)
                .map(|value| value.0)
                .unwrap_or(320.)),
        }
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Options {
    #[serde(default)]
    skip_tests: bool,
    #[serde(default)]
    offline: bool,
    #[serde(default)]
    also_make: bool,
    #[serde(default)]
    enabled_profiles: BTreeSet<String>,
    #[serde(default)]
    disabled_profiles: BTreeSet<String>,
}

#[derive(Clone)]
enum RowKind {
    Module,
    Group,
    Goal(String),
    Profile { id: String, default_active: bool },
    Dependency,
}
#[derive(Clone)]
struct Row {
    key: String,
    module: usize,
    depth: usize,
    label: String,
    kind: RowKind,
}

pub struct MavenPanel {
    project: Entity<Project>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    search: Entity<Editor>,
    model: MavenModel,
    options: Options,
    expanded: BTreeSet<String>,
    selected: Option<String>,
    storage_key: Option<String>,
    loading: bool,
    error: Option<String>,
    scan_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

pub fn init(cx: &mut App) {
    MavenPanelSettings::register(cx);
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<MavenPanel>(window, cx);
        });
    })
    .detach();
}

impl MavenPanel {
    pub async fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        let storage_key = workspace.read_with(&cx, |workspace, _| {
            workspace
                .database_id()
                .map(|id| format!("maven-panel-{id:?}"))
        })?;
        let options = if let Some(key) = storage_key.clone() {
            let store = cx.update(|_, cx| KeyValueStore::global(cx))?;
            cx.background_spawn(async move { store.read_kvp(&key) })
                .await
                .log_err()
                .flatten()
                .and_then(|value| serde_json::from_str(&value).log_err())
                .unwrap_or_default()
        } else {
            Options::default()
        };
        workspace.update_in(&mut cx, |workspace, window, cx| {
            let project = workspace.project().clone();
            let workspace = cx.entity().downgrade();
            cx.new(|cx| {
                let search = cx.new(|cx| {
                    let mut editor = Editor::single_line(window, cx);
                    editor.set_placeholder_text("搜索", window, cx);
                    editor
                });
                let subscriptions = vec![
                    cx.subscribe(&project, |this: &mut Self, _, event, cx| {
                        let refresh = match event {
                            project::Event::WorktreeAdded(_)
                            | project::Event::WorktreeRemoved(_) => true,
                            project::Event::WorktreeUpdatedEntries(_, changes) => {
                                changes.iter().any(|(path, _, _)| {
                                    path.as_unix_str() == "pom.xml"
                                        || path.as_unix_str().ends_with("/pom.xml")
                                })
                            }
                            _ => false,
                        };
                        if refresh {
                            this.refresh(cx);
                        } else {
                            cx.notify();
                        }
                    }),
                    cx.subscribe(&search, |_: &mut Self, _, event, cx| {
                        if matches!(event, EditorEvent::BufferEdited) {
                            cx.notify();
                        }
                    }),
                    cx.observe_global::<SettingsStore>(|_, cx| cx.notify()),
                ];
                let mut panel = Self {
                    project,
                    workspace,
                    focus: cx.focus_handle(),
                    search,
                    model: MavenModel::default(),
                    options,
                    expanded: BTreeSet::new(),
                    selected: None,
                    storage_key,
                    loading: false,
                    error: None,
                    scan_task: None,
                    _subscriptions: subscriptions,
                };
                panel.refresh(cx);
                panel
            })
        })
    }

    fn save_options(&self, cx: &mut Context<Self>) {
        let Some(key) = self.storage_key.clone() else {
            return;
        };
        let Some(value) = serde_json::to_string(&self.options).log_err() else {
            return;
        };
        let store = KeyValueStore::global(cx);
        cx.spawn(async move |_, _| store.write_kvp(key, value).await)
            .detach_and_log_err(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let roots = self
            .project
            .read(cx)
            .visible_worktrees(cx)
            .filter_map(|worktree| {
                let worktree = worktree.read(cx);
                if !worktree.is_local() {
                    return None;
                }
                let poms = worktree
                    .snapshot()
                    .files(false, 0)
                    .filter(|entry| {
                        entry.path.as_unix_str() == "pom.xml"
                            || entry.path.as_unix_str().ends_with("/pom.xml")
                    })
                    .take(512)
                    .map(|entry| PathBuf::from(entry.path.as_unix_str()))
                    .collect();
                Some(ScanRoot {
                    id: worktree.id(),
                    path: worktree.abs_path().to_path_buf(),
                    poms,
                })
            })
            .collect();
        self.loading = true;
        self.error = None;
        self.scan_task = Some(cx.spawn(async move |this, cx| {
            let model = cx.background_spawn(async move { model::scan(roots) }).await;
            this.update(cx, |this, cx| {
                for root in &model.roots {
                    if let Some(module) = model.modules.get(*root) {
                        if !this
                            .model
                            .modules
                            .iter()
                            .any(|previous| previous.path == module.path)
                        {
                            let key = module.path.to_string_lossy().into_owned();
                            this.expanded.insert(key.clone());
                            this.expanded.insert(format!("{key}/Lifecycle"));
                        }
                    }
                }
                this.model = model;
                this.loading = false;
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let query = self.search.read(cx).text(cx).trim().to_lowercase();
        let mut rows = Vec::new();
        let mut visited = BTreeSet::new();
        for root in &self.model.roots {
            self.module_rows(*root, 0, &query, &mut visited, &mut rows);
        }
        if query.is_empty() {
            return rows;
        }
        let mut keep = vec![false; rows.len()];
        let mut ancestors: Vec<usize> = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            while ancestors
                .last()
                .is_some_and(|parent| rows[*parent].depth >= row.depth)
            {
                ancestors.pop();
            }
            let branch = matches!(row.kind, RowKind::Module | RowKind::Group);
            if !branch || row.label.to_lowercase().contains(&query) {
                keep[index] = true;
                for parent in &ancestors {
                    keep[*parent] = true;
                }
            }
            if branch {
                ancestors.push(index);
            }
        }
        rows.into_iter()
            .zip(keep)
            .filter_map(|(row, keep)| keep.then_some(row))
            .collect()
    }

    fn module_rows(
        &self,
        index: usize,
        depth: usize,
        query: &str,
        visited: &mut BTreeSet<usize>,
        rows: &mut Vec<Row>,
    ) {
        if depth > 32 || !visited.insert(index) {
            return;
        }
        let Some(module) = self.model.modules.get(index) else {
            return;
        };
        let key = module.path.to_string_lossy().into_owned();
        let searching = !query.is_empty();
        rows.push(Row {
            key: key.clone(),
            module: index,
            depth,
            label: format!("{} ({})", module.pom.name, module.pom.packaging),
            kind: RowKind::Module,
        });
        if !searching && !self.expanded.contains(&key) {
            return;
        }
        let matches = |label: &str| {
            !searching
                || label.to_lowercase().contains(query)
                || module.pom.name.to_lowercase().contains(query)
        };
        let mut group = |name: &str, entries: Vec<(String, RowKind)>| {
            let group_key = format!("{key}/{name}");
            rows.push(Row {
                key: group_key.clone(),
                module: index,
                depth: depth + 1,
                label: name.into(),
                kind: RowKind::Group,
            });
            if searching || self.expanded.contains(&group_key) {
                for (label, kind) in entries {
                    if matches(&label) {
                        rows.push(Row {
                            key: format!("{group_key}/{label}"),
                            module: index,
                            depth: depth + 2,
                            label,
                            kind,
                        });
                    }
                }
            }
        };
        group(
            "Lifecycle",
            PHASES
                .iter()
                .map(|phase| ((*phase).into(), RowKind::Goal((*phase).into())))
                .collect(),
        );
        if !module.pom.profiles.is_empty() {
            group(
                "Profiles",
                module
                    .pom
                    .profiles
                    .iter()
                    .map(|profile| {
                        (
                            profile.id.clone(),
                            RowKind::Profile {
                                id: profile.id.clone(),
                                default_active: profile.active_by_default,
                            },
                        )
                    })
                    .collect(),
            );
        }
        group(
            "Dependencies",
            std::iter::once((
                "dependency:tree".into(),
                RowKind::Goal("dependency:tree".into()),
            ))
            .chain(
                module
                    .pom
                    .dependencies
                    .iter()
                    .map(|dependency| (dependency.clone(), RowKind::Dependency)),
            )
            .collect(),
        );
        let plugin_key = format!("{key}/Plugins");
        rows.push(Row {
            key: plugin_key.clone(),
            module: index,
            depth: depth + 1,
            label: "Plugins".into(),
            kind: RowKind::Group,
        });
        if searching || self.expanded.contains(&plugin_key) {
            for plugin in &module.pom.plugins {
                let plugin_key = format!("{plugin_key}/{}", plugin.coordinate);
                rows.push(Row {
                    key: plugin_key.clone(),
                    module: index,
                    depth: depth + 2,
                    label: plugin.name.clone(),
                    kind: RowKind::Group,
                });
                if searching || self.expanded.contains(&plugin_key) {
                    for goal in &plugin.goals {
                        if matches(goal) || matches(&plugin.name) {
                            rows.push(Row {
                                key: format!("{plugin_key}/{goal}"),
                                module: index,
                                depth: depth + 3,
                                label: goal.clone(),
                                kind: RowKind::Goal(format!("{}:{goal}", plugin.coordinate)),
                            });
                        }
                    }
                    if plugin.goals.is_empty()
                        && (matches(&plugin.name) || matches("help:describe"))
                    {
                        rows.push(Row {
                            key: format!("{plugin_key}/describe"),
                            module: index,
                            depth: depth + 3,
                            label: "查看插件目标（help:describe）".into(),
                            kind: RowKind::Goal(format!(
                                "help:describe,-Dplugin={},-Ddetail",
                                plugin.coordinate
                            )),
                        });
                    }
                }
            }
        }
        for child in &module.children {
            self.module_rows(*child, depth + 1, query, visited, rows);
        }
    }

    fn current_module_index(&self, row: &Row) -> Option<usize> {
        self.model.modules.iter().position(|module| {
            let path = module.path.to_string_lossy();
            row.key == path || row.key.starts_with(&format!("{path}/"))
        })
    }

    fn profile_enabled(&self, index: usize, id: &str, default_active: bool) -> bool {
        let Some(module) = self.model.modules.get(index) else {
            return false;
        };
        let prefix = format!("{}#", module.reactor.display());
        let key = format!("{prefix}{id}");
        let explicit = module.pom.profiles.iter().any(|profile| {
            self.options
                .enabled_profiles
                .contains(&format!("{prefix}{}", profile.id))
        });
        self.options.enabled_profiles.contains(&key)
            || (default_active && !explicit && !self.options.disabled_profiles.contains(&key))
    }

    fn selected_row(&self, cx: &App) -> Option<Row> {
        self.rows(cx)
            .into_iter()
            .find(|row| Some(&row.key) == self.selected.as_ref())
    }
    fn run_selected(&mut self, _: &RunSelected, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Row {
            module,
            kind: RowKind::Goal(goal),
            ..
        }) = self.selected_row(cx)
        {
            self.run(module, goal, window, cx);
        }
    }
    fn move_selection(&mut self, direction: isize, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        if rows.is_empty() {
            return;
        }
        let index = rows
            .iter()
            .position(|row| Some(&row.key) == self.selected.as_ref())
            .map(|index| (index as isize + direction).clamp(0, rows.len() as isize - 1) as usize)
            .unwrap_or(0);
        self.selected = rows.get(index).map(|row| row.key.clone());
        cx.notify();
    }
    fn run(&mut self, index: usize, goal: String, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.prepare_task(index, &goal, cx);
        match result {
            Ok((template, context, worktree)) => {
                if let Some(workspace) = self.workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.schedule_task(
                            TaskSourceKind::Worktree {
                                id: worktree,
                                directory_in_worktree: util::rel_path::RelPath::empty().into(),
                                id_base: "maven-panel".into(),
                            },
                            &template,
                            &context,
                            false,
                            window,
                            cx,
                        )
                    });
                    self.error = None;
                }
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }
    fn prepare_task(
        &self,
        index: usize,
        goal: &str,
        cx: &App,
    ) -> Result<(TaskTemplate, TaskContext, worktree::WorktreeId)> {
        ensure!(
            !TrustedWorktrees::has_restricted_worktrees(
                &self.project.read(cx).worktree_store(),
                cx
            ),
            "信任项目后才能执行 Maven"
        );
        let module = self
            .model
            .modules
            .get(index)
            .context("模块已移除，请刷新")?;
        ensure!(
            !self
                .project
                .read(cx)
                .has_running_task_with_tag(module.worktree, TASK_TAG, cx),
            "Maven 任务正在运行；请等待或停止任务"
        );
        let read_config = |name: &str| -> Result<serde_json::Value> {
            let path = module.root.join(name);
            if !path.is_file() {
                return Ok(serde_json::Value::Null);
            }
            let text = std::fs::read_to_string(path).context("无法读取 Maven 运行配置")?;
            Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))
                .context("Maven 运行配置不是有效 JSON")?)
        };
        let config = read_config(".zed/run-environments.json")?;
        let settings = &config["environments"]["java"];
        let reactor = module.reactor.parent().context("Reactor 目录无效")?;
        let mut build_tool = settings["build_tool"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| reactor.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }));
        if build_tool.is_dir() {
            let names: &[&str] = if cfg!(windows) {
                &["bin/mvnd.exe", "bin/mvn.cmd", "mvnd.exe", "mvn.cmd"]
            } else {
                &["bin/mvnd", "bin/mvn", "mvnd", "mvn"]
            };
            if let Some(path) = names
                .iter()
                .map(|name| build_tool.join(name))
                .find(|path| path.is_file())
            {
                build_tool = path;
            }
        }
        if !build_tool.is_file() {
            let wrapper = reactor.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" });
            if wrapper.is_file() {
                build_tool = wrapper;
            }
        }
        if !build_tool.is_file() {
            let names: &[&str] = if cfg!(windows) {
                &["mvnd.exe", "mvn.cmd"]
            } else {
                &["mvnd", "mvn"]
            };
            let directories = std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
                .unwrap_or_default();
            if let Some(path) = names
                .iter()
                .flat_map(|name| {
                    directories
                        .iter()
                        .map(move |directory| directory.join(name))
                })
                .find(|path| path.is_file())
            {
                build_tool = path;
            }
        }
        ensure!(
            build_tool.is_absolute() && build_tool.is_file(),
            "请配置有效的 java.build_tool 或项目 Maven Wrapper"
        );
        ensure!(
            module.path.starts_with(&module.root) && module.reactor.starts_with(&module.root),
            "只能运行当前项目的 Maven 模块"
        );
        let mut arguments = vec!["-f".to_string()];
        if self.options.also_make && module.path != module.reactor {
            let relative = module
                .path
                .parent()
                .context("模块目录无效")?
                .strip_prefix(reactor)
                .context("模块不在 Reactor 内")?;
            arguments.extend([
                module.reactor.to_string_lossy().into_owned(),
                "-pl".into(),
                relative.to_string_lossy().replace('\\', "/"),
                "-am".into(),
            ]);
        } else {
            arguments.push(module.path.to_string_lossy().into_owned());
        }
        arguments.push("-B".into());
        let prefix = format!("{}#", module.reactor.display());
        let profiles: Vec<_> = self
            .options
            .enabled_profiles
            .iter()
            .filter_map(|profile| profile.strip_prefix(&prefix).map(str::to_owned))
            .chain(self.options.disabled_profiles.iter().filter_map(|profile| {
                profile
                    .strip_prefix(&prefix)
                    .map(|profile| format!("!{profile}"))
            }))
            .collect();
        if !profiles.is_empty() {
            arguments.extend(["-P".into(), profiles.join(",")]);
        }
        if self.options.skip_tests {
            arguments.push("-DskipTests".into());
        }
        if self.options.offline {
            arguments.push("-o".into());
        }
        for goal in goal.split(',').filter(|goal| !goal.is_empty()) {
            ensure!(
                (!goal.starts_with('-')
                    && goal
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric()
                            || "_.:-".contains(character)))
                    || goal == "-Ddetail"
                    || goal
                        .strip_prefix("-Dplugin=")
                        .is_some_and(|value| !value.is_empty()
                            && value
                                .chars()
                                .all(|character| character.is_ascii_alphanumeric()
                                    || "_.:-".contains(character))),
                "Maven 目标无效"
            );
            arguments.push(goal.into());
        }
        let kind = if cfg!(windows) {
            task::ShellKind::Cmd
        } else {
            task::ShellKind::Posix
        };
        let quote = |value: &str| {
            kind.try_quote(value)
                .map(|value| value.into_owned())
                .context("Maven 参数无法由系统 Shell 表达")
        };
        let mut template = TaskTemplate {
            label: format!("Maven · {} · {goal}", module.pom.name),
            command: quote(&build_tool.to_string_lossy())?,
            args: arguments
                .iter()
                .map(|argument| quote(argument))
                .collect::<Result<Vec<_>>>()?,
            cwd: Some(reactor.to_string_lossy().into_owned()),
            tags: vec![TASK_TAG.into()],
            shell: task::Shell::Program(if cfg!(windows) {
                std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
            } else {
                "/bin/sh".into()
            }),
            show_command: false,
            allow_concurrent_runs: false,
            use_new_terminal: false,
            ..Default::default()
        };
        let private = read_config(".zed/run-config.json")?;
        for variables in [private["env"].as_object(), settings["env"].as_object()]
            .into_iter()
            .flatten()
        {
            for (key, value) in variables {
                let value = match value {
                    serde_json::Value::String(value) => value.clone(),
                    serde_json::Value::Number(_) | serde_json::Value::Bool(_) => value.to_string(),
                    serde_json::Value::Null => String::new(),
                    _ => anyhow::bail!("Maven 环境变量 {key} 必须为字符串或标量"),
                };
                template.env.insert(key.clone(), value);
            }
        }
        if let Some(java_home) = settings["runtime"]
            .as_str()
            .map(PathBuf::from)
            .filter(|path| {
                path.is_absolute()
                    && path
                        .join(if cfg!(windows) {
                            "bin/java.exe"
                        } else {
                            "bin/java"
                        })
                        .is_file()
            })
        {
            template
                .env
                .insert("JAVA_HOME".into(), java_home.to_string_lossy().into_owned());
            let path_key = template
                .env
                .keys()
                .find(|key| key.eq_ignore_ascii_case("PATH"))
                .cloned()
                .unwrap_or_else(|| "PATH".into());
            let original = template
                .env
                .get(&path_key)
                .cloned()
                .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
            let joined = std::env::join_paths(
                std::iter::once(java_home.join("bin")).chain(std::env::split_paths(&original)),
            )?;
            template
                .env
                .insert(path_key, joined.to_string_lossy().into_owned());
        }
        Ok((
            template,
            TaskContext {
                cwd: Some(module.root.clone()),
                ..Default::default()
            },
            module.worktree,
        ))
    }
    fn open_pom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self
            .selected_row(cx)
            .map(|row| row.module)
            .or_else(|| self.model.roots.first().copied());
        if let Some(path) = index
            .and_then(|index| self.model.modules.get(index))
            .map(|module| module.path.clone())
            && let Some(workspace) = self.workspace.upgrade()
        {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.open_abs_path(path, Default::default(), window, cx)
                })
                .detach_and_log_err(cx);
        }
    }
}

impl Focusable for MavenPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl EventEmitter<PanelEvent> for MavenPanel {}
impl Panel for MavenPanel {
    fn persistent_name() -> &'static str {
        "Maven"
    }
    fn panel_key() -> &'static str {
        "maven_panel"
    }
    fn position(&self, _: &Window, cx: &App) -> DockPosition {
        match MavenPanelSettings::get_global(cx).dock {
            DockSide::Left => DockPosition::Left,
            DockSide::Right => DockPosition::Right,
        }
    }
    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }
    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        settings::update_settings_file(
            self.project.read(cx).fs().clone(),
            cx,
            move |settings, _| {
                settings.maven_panel.get_or_insert_default().dock =
                    Some(if position == DockPosition::Left {
                        DockSide::Left
                    } else {
                        DockSide::Right
                    });
            },
        );
    }
    fn default_size(&self, _: &Window, cx: &App) -> Pixels {
        MavenPanelSettings::get_global(cx).width
    }
    fn icon(&self, _: &Window, cx: &App) -> Option<IconName> {
        (MavenPanelSettings::get_global(cx).button
            && (!self.model.modules.is_empty() || !self.model.warnings.is_empty()))
        .then_some(IconName::Maven)
    }
    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Maven")
    }
    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }
    fn starts_open(&self, _: &Window, cx: &App) -> bool {
        MavenPanelSettings::get_global(cx).starts_open
    }
    fn activation_priority(&self) -> u32 {
        7
    }
    fn hide_button_setting(&self, _: &App) -> Option<workspace::HideStatusItem> {
        Some(workspace::HideStatusItem::new(|settings| {
            settings.maven_panel.get_or_insert_default().button = Some(false);
        }))
    }
}

impl Render for MavenPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let searching = !self.search.read(cx).text(cx).trim().is_empty();
        let selected_goal = self
            .selected_row(cx)
            .is_some_and(|row| matches!(row.kind, RowKind::Goal(_)));
        let busy = self.model.modules.iter().any(|module| {
            self.project
                .read(cx)
                .has_running_task_with_tag(module.worktree, TASK_TAG, cx)
        });
        let status = self
            .error
            .clone()
            .or_else(|| self.model.warnings.first().cloned());
        v_flex()
            .id("maven-panel")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus)
            .key_context("MavenPanel")
            .on_action(cx.listener(Self::run_selected))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &Expand, _, cx| {
                if let Some(row) = this.selected_row(cx) {
                    this.expanded.insert(row.key);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Collapse, _, cx| {
                if let Some(row) = this.selected_row(cx) {
                    this.expanded.remove(&row.key);
                    cx.notify();
                }
            }))
            .child(
                h_flex()
                    .px_3()
                    .h_9()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new("Maven").weight(gpui::FontWeight::SEMIBOLD))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                IconButton::new("refresh", IconName::RotateCw)
                                    .disabled(self.loading)
                                    .tooltip(Tooltip::text("重新读取 Maven 项目（不执行构建）"))
                                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                            )
                            .child(
                                IconButton::new("run", IconName::PlayFilled)
                                    .disabled(!selected_goal || busy)
                                    .tooltip(Tooltip::text("执行选中的 Maven 目标"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.run_selected(&RunSelected, window, cx)
                                    })),
                            )
                            .child(
                                IconButton::new("stop", IconName::Stop)
                                    .disabled(!busy)
                                    .tooltip(Tooltip::text("停止 Maven 任务"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        for module in &this.model.modules {
                                            this.project.update(cx, |project, cx| {
                                                project.stop_tasks_with_tag(
                                                    module.worktree,
                                                    TASK_TAG,
                                                    cx,
                                                )
                                            });
                                        }
                                    })),
                            )
                            .child(
                                IconButton::new("pom", IconName::FileCode)
                                    .tooltip(Tooltip::text("打开所选模块的 pom.xml"))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.open_pom(window, cx)
                                        }),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        IconButton::new("skip-tests", IconName::Check)
                            .tooltip(Tooltip::text("跳过测试"))
                            .toggle_state(self.options.skip_tests)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.options.skip_tests = !this.options.skip_tests;
                                this.save_options(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        IconButton::new("offline", IconName::Disconnected)
                            .tooltip(Tooltip::text("离线"))
                            .toggle_state(self.options.offline)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.options.offline = !this.options.offline;
                                this.save_options(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        IconButton::new("also-make", IconName::ListTree)
                            .tooltip(Tooltip::text("同时构建依赖模块"))
                            .toggle_state(self.options.also_make)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.options.also_make = !this.options.also_make;
                                this.save_options(cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(self.search.clone()),
            )
            .when(self.loading, |this| {
                this.child(
                    div().px_3().py_1().child(
                        Label::new("正在读取 POM…")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
                )
            })
            .when_some(status, |this, status| {
                this.child(
                    div().px_3().py_2().child(
                        Label::new(status)
                            .size(LabelSize::Small)
                            .color(Color::Warning),
                    ),
                )
            })
            .child(
                v_flex()
                    .id("maven-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py_1()
                    .when(rows.is_empty() && !self.loading, |this| {
                        this.child(
                            div().p_3().child(
                                Label::new(if self.model.modules.is_empty() {
                                    "工作区中未发现 Maven 项目。打开含 pom.xml 的项目后刷新。"
                                } else {
                                    "没有匹配的模块或目标。"
                                })
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                            ),
                        )
                    })
                    .children(rows.into_iter().map(|row| {
                        let selected = self.selected.as_ref() == Some(&row.key);
                        let expandable = matches!(row.kind, RowKind::Module | RowKind::Group);
                        let expanded = searching || self.expanded.contains(&row.key);
                        let icon = match &row.kind {
                            RowKind::Module => IconName::Box,
                            RowKind::Group => IconName::Folder,
                            RowKind::Goal(_) => IconName::PlayOutlined,
                            RowKind::Dependency => IconName::Link,
                            RowKind::Profile { id, default_active } => {
                                if self.profile_enabled(row.module, id, *default_active) {
                                    IconName::Check
                                } else {
                                    IconName::Circle
                                }
                            }
                        };
                        ListItem::new(row.key.clone())
                            .tooltip(Tooltip::text(format!(
                                "{}\n{}",
                                self.model.modules[row.module].pom.coordinate,
                                self.model.modules[row.module].path.display()
                            )))
                            .indent_level(row.depth)
                            .indent_step_size(px(14.))
                            .toggle_state(selected)
                            .child(
                                h_flex()
                                    .h_6()
                                    .gap_2()
                                    .min_w_0()
                                    .when(expandable, |row| {
                                        row.child(
                                            Icon::new(if expanded {
                                                IconName::ChevronDown
                                            } else {
                                                IconName::ChevronRight
                                            })
                                            .size(IconSize::Small)
                                            .color(Color::Muted),
                                        )
                                    })
                                    .when(!expandable, |row| row.child(div().w(px(14.))))
                                    .child(Icon::new(icon).size(IconSize::Small).color(
                                        if matches!(row.kind, RowKind::Goal(_)) {
                                            Color::Accent
                                        } else {
                                            Color::Muted
                                        },
                                    ))
                                    .child(
                                        Label::new(row.label.clone())
                                            .size(LabelSize::Small)
                                            .truncate(),
                                    ),
                            )
                            .on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, window, cx| {
                                    window.focus(&this.focus, cx);
                                    let Some(module_index) = this.current_module_index(&row) else {
                                        return;
                                    };
                                    this.selected = Some(row.key.clone());
                                    match &row.kind {
                                        RowKind::Goal(goal) if event.click_count() > 1 => {
                                            this.run(module_index, goal.clone(), window, cx)
                                        }
                                        RowKind::Profile { id, default_active } => {
                                            let key = format!(
                                                "{}#{id}",
                                                this.model.modules[module_index].reactor.display()
                                            );
                                            let enabled = this.profile_enabled(
                                                module_index,
                                                id,
                                                *default_active,
                                            );
                                            if enabled {
                                                this.options.enabled_profiles.remove(&key);
                                                this.options.disabled_profiles.insert(key);
                                            } else {
                                                this.options.disabled_profiles.remove(&key);
                                                this.options.enabled_profiles.insert(key);
                                            }
                                            this.save_options(cx);
                                        }
                                        _ if expandable => {
                                            if !this.expanded.remove(&row.key) {
                                                this.expanded.insert(row.key.clone());
                                            }
                                        }
                                        _ => {}
                                    }
                                    cx.notify();
                                },
                            ))
                    })),
            )
            .when(busy, |this| {
                this.child(
                    h_flex()
                        .px_3()
                        .h_7()
                        .border_t_1()
                        .border_color(cx.theme().colors().border_variant)
                        .child(
                            Label::new("Maven 正在运行")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                )
            })
    }
}
