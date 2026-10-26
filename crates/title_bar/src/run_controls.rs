use super::*;
use crate::run_entries::{self, RunEntry};
use std::path::PathBuf;
use task::{TaskContext, VariableName};
use util::rel_path::RelPath;

#[derive(Default)]
pub(super) struct RunControls {
    root: Option<PathBuf>,
    entries: Vec<RunEntry>,
    selected: Option<String>,
    active_file: Option<PathBuf>,
    error: Option<String>,
    scan_id: usize,
    scan: Option<gpui::Task<()>>,
    transition: bool,
    launch: Option<gpui::Task<()>>,
}

impl TitleBar {
    fn refresh_run_entries(&mut self, cx: &mut Context<Self>) {
        let Some(worktree) = self.effective_active_worktree(cx) else {
            return;
        };
        let worktree = worktree.read(cx);
        if !worktree.is_local() {
            return;
        }
        let root = worktree.abs_path().to_path_buf();
        self.run_controls.scan_id = worktree.completed_scan_id();
        self.run_controls.root = Some(root.clone());
        self.run_controls.error = None;
        if TrustedWorktrees::has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx) {
            self.run_controls.entries.clear();
            self.run_controls.error = Some("信任项目后才能加载运行入口扩展".into());
            return;
        }
        let discovery = cx.background_spawn(async move { run_entries::discover(&root).await });
        self.run_controls.scan = Some(cx.spawn(async move |this, cx| {
            let result = discovery.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(discovery) => {
                        this.run_controls.entries = discovery.entries;
                        this.run_controls.active_file = None;
                        this.run_controls.error = discovery.warnings.first().cloned();
                    }
                    Err(error) => {
                        this.run_controls.entries.clear();
                        this.run_controls.error = Some(format!("{error:#}"));
                    }
                }
                this.run_controls.scan = None;
                cx.notify();
            })
            .log_err();
        }));
    }

    fn select_run_entry(&mut self, id: String, cx: &mut Context<Self>) {
        self.run_controls.selected = Some(id.clone());
        if let Some(root) = &self.run_controls.root {
            let key = format!("run-entry:{}", root.display());
            let store = db::kvp::KeyValueStore::global(cx);
            cx.spawn(async move |_, _| store.write_kvp(key, id).await)
                .detach_and_log_err(cx);
        }
        cx.notify();
    }

    fn open_run_entry_source(
        &mut self,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(root), Some(workspace)) = (&self.run_controls.root, self.workspace.upgrade())
        else {
            return;
        };
        let path = root.join(relative_path);
        let task = workspace.update(cx, |workspace, cx| {
            workspace.open_abs_path(path, Default::default(), window, cx)
        });
        task.detach_and_notify_err(workspace.downgrade(), window, cx);
    }

    pub(super) fn render_run_controls(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let worktree = self.effective_active_worktree(cx)?;
        let tree = worktree.read(cx);
        if !tree.is_local() {
            return None;
        }
        let root = tree.abs_path().to_path_buf();
        let worktree_id = tree.id();
        if self.run_controls.root.as_ref() != Some(&root) {
            self.run_controls = RunControls::default();
            self.run_controls.selected = db::kvp::KeyValueStore::global(cx)
                .read_kvp(&format!("run-entry:{}", root.display()))
                .log_err()
                .flatten();
            self.refresh_run_entries(cx);
        } else if self.run_controls.scan.is_none()
            && self.run_controls.entries.is_empty()
            && self.run_controls.scan_id != tree.completed_scan_id()
        {
            self.refresh_run_entries(cx);
        }
        let active_file = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).active_item(cx))
            .and_then(|item| item.project_path(cx))
            .filter(|path| path.worktree_id == worktree_id)
            .map(|path| path.path.as_std_path().to_path_buf());
        if self.run_controls.active_file != active_file {
            if let Some(active_file) = &active_file {
                let extension = active_file
                    .extension()
                    .and_then(|extension| extension.to_str());
                let entries = &self.run_controls.entries;
                let current_language = entries
                    .iter()
                    .find(|entry| Some(&entry.id) == self.run_controls.selected.as_ref())
                    .map(|entry| entry.language.as_str());
                let candidate = entries
                    .iter()
                    .find(|entry| &entry.file == active_file)
                    .or_else(|| {
                        extension.and_then(|extension| {
                            entries.iter().find(|entry| {
                                Some(entry.language.as_str()) != current_language
                                    && entry
                                        .matching_extensions
                                        .iter()
                                        .any(|candidate| candidate == extension)
                            })
                        })
                    });
                if let Some(candidate) = candidate {
                    self.run_controls.selected = Some(candidate.id.clone());
                }
            }
            self.run_controls.active_file = active_file;
        }
        let selected = self
            .run_controls
            .entries
            .iter()
            .find(|entry| Some(&entry.id) == self.run_controls.selected.as_ref())
            .or_else(|| self.run_controls.entries.first())
            .cloned();
        let tag = selected
            .as_ref()
            .map(|entry| format!("{}:{}", run_entries::RUN_TAG, entry.id));
        let running = tag.as_ref().is_some_and(|tag| {
            self.project
                .read(cx)
                .has_running_task_with_tag(worktree_id, tag, cx)
        });
        let restricted =
            TrustedWorktrees::has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx);
        let busy = self.run_controls.transition;
        let label = selected
            .as_ref()
            .map(|entry| entry.task.label.clone())
            .unwrap_or_else(|| {
                if self.run_controls.scan.is_some() {
                    "扫描启动入口…".into()
                } else {
                    "选择启动入口".into()
                }
            });
        let this = cx.entity().downgrade();
        let debug_entry = selected
            .as_ref()
            .filter(|entry| entry.debug.is_some())
            .cloned();
        let entries = self.run_controls.entries.clone();
        let error = self.run_controls.error.clone();
        let selected_id = selected.as_ref().map(|entry| entry.id.clone());
        let selector = PopoverMenu::new("run-entry-picker")
            .trigger(
                ButtonLike::new("run-entry-picker-trigger")
                    .child(Label::new(label).size(LabelSize::Small))
                    .child(Icon::new(IconName::ChevronDown).size(IconSize::XSmall)),
            )
            .menu(move |window, cx| {
                Some(ContextMenu::build(window, cx, |mut menu, _, _| {
                    menu = menu
                        .end_slot_action(Box::new(menu::SecondaryConfirm))
                        .header("项目启动入口");
                    for entry in &entries {
                        let id = entry.id.clone();
                        let this = this.clone();
                        let label = format!(
                            "{}{}",
                            if selected_id.as_ref() == Some(&id) {
                                "✓ "
                            } else {
                                ""
                            },
                            entry.task.label
                        );
                        if entry.file.as_os_str().is_empty() {
                            menu = menu.entry(label, None, move |_, cx| {
                                this.update(cx, |this, cx| this.select_run_entry(id.clone(), cx))
                                    .log_err();
                            });
                        } else {
                            let source_path = entry.file.clone();
                            let source_title_bar = this.clone();
                            menu = menu.entry_with_end_slot(
                                label,
                                None,
                                move |_, cx| {
                                    this.update(cx, |this, cx| {
                                        this.select_run_entry(id.clone(), cx)
                                    })
                                    .log_err();
                                },
                                IconName::ArrowUpRight,
                                "定位入口代码".into(),
                                move |window, cx| {
                                    source_title_bar
                                        .update(cx, |this, cx| {
                                            this.open_run_entry_source(
                                                source_path.clone(),
                                                window,
                                                cx,
                                            )
                                        })
                                        .log_err();
                                },
                            );
                        }
                    }
                    if let Some(error) = &error {
                        menu = menu.header(error.clone());
                    }
                    menu =
                        menu.separator()
                            .entry("重新扫描入口 / 重载运行环境", None, {
                                let this = this.clone();
                                move |_, cx| {
                                    this.update(cx, |this, cx| this.refresh_run_entries(cx))
                                        .log_err();
                                }
                            });
                    menu.entry(
                        "编辑运行环境（JDK / Python / 环境变量）",
                        None,
                        {
                            let this = this.clone();
                            move |window, cx| {
                                this.update(cx, |this, cx| {
                                    if let (Some(root), Some(workspace)) =
                                        (&this.run_controls.root, this.workspace.upgrade())
                                    {
                                        let path = root.join(run_entries::CONFIG_PATH);
                                        let task_workspace = workspace.clone();
                                        let task = cx.spawn_in(window, async move |_, cx| {
                                            let config_path = path.clone();
                                            cx.background_spawn(async move {
                                                use std::io::Write as _;
                                                if let Some(parent) = config_path.parent() { std::fs::create_dir_all(parent)?; }
                                                let template = run_entries::config_template()?;
                                                match std::fs::OpenOptions::new().write(true).create_new(true).open(config_path) {
                                                    Ok(mut file) => file.write_all(template.as_bytes())?,
                                                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
                                                    Err(error) => return Err(error.into()),
                                                }
                                                anyhow::Ok(())
                                            }).await?;
                                            task_workspace.update_in(cx, |workspace, window, cx| workspace.open_abs_path(path, Default::default(), window, cx))?.await?;
                                            anyhow::Ok(())
                                        });
                                        task.detach_and_notify_err(workspace.downgrade(), window, cx);
                                    }
                                })
                                .log_err();
                            }
                        },
                    )
                }))
            });
        Some(
            h_flex()
                .gap_1()
                .child(selector)
                .child(
                    IconButton::new(
                        "run-selected-entry",
                        if running {
                            IconName::Rerun
                        } else {
                            IconName::PlayFilled
                        },
                    )
                    .icon_size(IconSize::Small)
                    .icon_color(Color::Success)
                    .disabled(selected.is_none() || busy || restricted)
                    .tooltip(Tooltip::text(if running {
                        "重新运行选中入口"
                    } else {
                        "运行选中入口"
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(entry) = selected.clone() {
                            this.launch_run_entry(entry, worktree_id, running, window, cx);
                        }
                    })),
                )
                .child(
                    IconButton::new("debug-selected-entry", IconName::Debug)
                        .icon_size(IconSize::Small)
                        .disabled(debug_entry.is_none() || restricted || busy)
                        .tooltip(Tooltip::text("调试选中入口"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(entry) = debug_entry.clone() {
                                this.debug_run_entry(entry, worktree_id, window, cx);
                            }
                        })),
                )
                .child(
                    IconButton::new("stop-selected-entry", IconName::Stop)
                        .icon_size(IconSize::Small)
                        .icon_color(Color::Error)
                        .disabled(!running || busy)
                        .tooltip(Tooltip::text("停止选中入口"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(tag) = &tag {
                                this.project.update(cx, |project, cx| {
                                    project.stop_tasks_with_tag(worktree_id, tag, cx)
                                });
                            }
                        })),
                )
                .into_any_element(),
        )
    }

    fn launch_run_entry(
        &mut self,
        mut entry: RunEntry,
        worktree_id: project::WorktreeId,
        restart: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if TrustedWorktrees::has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx) {
            return;
        }
        let Some(root) = self.run_controls.root.clone() else {
            return;
        };
        let tag = format!("{}:{}", run_entries::RUN_TAG, entry.id);
        if restart {
            self.project.update(cx, |project, cx| {
                project.stop_tasks_with_tag(worktree_id, &tag, cx)
            });
        }
        self.run_controls.transition = true;
        let project = self.project.clone();
        let workspace = self.workspace.clone();
        entry.task.tags.push(tag.clone());
        entry.task.allow_concurrent_runs = false;
        entry.task.use_new_terminal = false;
        self.run_controls.launch = Some(cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let mut stopped = !restart;
                for _ in 0..100 {
                    if !project.read_with(cx, |project, cx| {
                        project.has_running_task_with_tag(worktree_id, &tag, cx)
                    }) {
                        stopped = true;
                        break;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                }
                if stopped {
                    workspace.update_in(cx, |workspace, window, cx| {
                        let mut task_context = TaskContext {
                            cwd: Some(root.clone()),
                            ..Default::default()
                        };
                        task_context.task_variables.insert(
                            VariableName::WorktreeRoot,
                            root.to_string_lossy().into_owned(),
                        );
                        workspace.schedule_task(
                            project::TaskSourceKind::Worktree {
                                id: worktree_id,
                                directory_in_worktree: Arc::from(RelPath::empty()),
                                id_base: "toolbar".into(),
                            },
                            &entry.task,
                            &task_context,
                            false,
                            window,
                            cx,
                        );
                    })?;
                    // Prevent double-click launches while save-for-task and terminal creation finish.
                    for _ in 0..100 {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        if project.read_with(cx, |project, cx| {
                            project.has_running_task_with_tag(worktree_id, &tag, cx)
                        }) {
                            break;
                        }
                    }
                }
                anyhow::ensure!(stopped, "进程尚未停止，请先检查终端，再重新运行");
                anyhow::Ok(())
            }
            .await;
            this.update(cx, |this, cx| {
                this.run_controls.transition = false;
                if let Err(error) = result {
                    this.run_controls.error = Some(format!("{error:#}"));
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn debug_run_entry(
        &mut self,
        entry: RunEntry,
        worktree_id: project::WorktreeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if TrustedWorktrees::has_restricted_worktrees(&self.project.read(cx).worktree_store(), cx) {
            return;
        }
        let (Some(root), Some(scenario), Some(workspace)) = (
            self.run_controls.root.clone(),
            entry.debug,
            self.workspace.upgrade(),
        ) else {
            return;
        };
        if workspace.read(cx).debugger_provider().is_none() {
            self.run_controls.error = Some("当前窗口未启用调试面板".into());
            cx.notify();
            return;
        }
        if dap::DapRegistry::global(cx)
            .adapter(&scenario.adapter)
            .is_none()
        {
            self.run_controls.error = Some(format!("尚未加载调试适配器：{}", scenario.adapter));
            cx.notify();
            return;
        }
        let mut task_context = TaskContext {
            cwd: Some(root.clone()),
            ..Default::default()
        };
        task_context.task_variables.insert(
            VariableName::WorktreeRoot,
            root.to_string_lossy().into_owned(),
        );
        workspace.update(cx, |workspace, cx| {
            workspace.start_debug_session(
                scenario,
                task_context.into(),
                None,
                Some(worktree_id),
                window,
                cx,
            );
        });
    }
}
