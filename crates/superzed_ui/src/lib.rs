use anyhow::Result;
use git_ui::git_panel::GitPanel;
use gpui::{
    App, AsyncWindowContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    PathPromptOptions, Pixels, PromptLevel, Render, Subscription, Task, WeakEntity, actions,
    prelude::FluentBuilder, px,
};
use project::DirectoryLister;
use project_panel::ProjectPanel;
use superzed_model::{ProjectEntry, SuperzedStore, TaskStatus, WorkspaceEntry, WorkspaceKind};
use ui::{Button, Color, Icon, IconButton, IconName, Indicator, Label, ListItem, prelude::*};
use util::ResultExt;
use workspace::{
    MultiWorkspace, MultiWorkspaceEvent, OpenOptions, OpenVisible, Sidebar as WorkspaceSidebar,
    SidebarEvent, Toast, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
    notifications::NotificationId,
};

actions!(
    superzed,
    [
        AddProject,
        NewWorkspace,
        LaunchAgent,
        RevealChanges,
        OpenWorkspaceInNewWindow,
        DeleteWorkspace,
        ToggleRightSidebar,
        CollapseWorkspaceSection,
        ExpandWorkspaceSection
    ]
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RightSidebarTab {
    Changes,
    Files,
}

pub fn init(cx: &mut App) {
    cx.observe_new(
        |workspace: &mut Workspace, _window, _: &mut Context<Workspace>| {
            workspace
                .register_action(|workspace, _: &AddProject, window, cx| {
                    run_add_project(workspace, window, cx);
                })
                .register_action(|workspace, _: &NewWorkspace, window, cx| {
                    run_new_workspace(workspace, window, cx);
                })
                .register_action(|workspace, _: &LaunchAgent, window, cx| {
                    run_launch_agent(workspace, window, cx);
                })
                .register_action(|workspace, _: &RevealChanges, window, cx| {
                    run_reveal_changes(workspace, window, cx);
                })
                .register_action(|workspace, _: &OpenWorkspaceInNewWindow, window, cx| {
                    run_open_workspace_in_new_window(workspace, window, cx);
                })
                .register_action(|workspace, _: &DeleteWorkspace, window, cx| {
                    run_delete_workspace(workspace, window, cx);
                })
                .register_action(|workspace, _: &ToggleRightSidebar, window, cx| {
                    if !workspace.toggle_panel_focus::<SuperzedRightSidebar>(window, cx) {
                        workspace.close_panel::<SuperzedRightSidebar>(window, cx);
                    }
                });
        },
    )
    .detach();
}

pub struct SuperzedSidebar {
    store: Entity<SuperzedStore>,
    multi_workspace: WeakEntity<MultiWorkspace>,
    focus_handle: FocusHandle,
    width: Option<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl SuperzedSidebar {
    pub fn new(
        multi_workspace: Entity<MultiWorkspace>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = SuperzedStore::global(cx);
        let weak_multi_workspace = multi_workspace.downgrade();
        let mut subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        subscriptions.push(cx.subscribe_in(
            &multi_workspace,
            window,
            |this, _, event, _, cx| match event {
                MultiWorkspaceEvent::ActiveWorkspaceChanged
                | MultiWorkspaceEvent::WorkspaceAdded(_)
                | MultiWorkspaceEvent::WorkspaceRemoved(_) => {
                    this.sync_active_workspace(cx);
                    cx.notify();
                }
            },
        ));

        let mut this = Self {
            store,
            multi_workspace: weak_multi_workspace,
            focus_handle: cx.focus_handle(),
            width: None,
            _subscriptions: subscriptions,
        };
        this.sync_active_workspace(cx);
        this
    }

    fn sync_active_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(current_workspace) = self.current_workspace_entity(cx) else {
            return;
        };
        let Some(path) = workspace_root_path(&current_workspace, cx) else {
            return;
        };
        self.store
            .update(cx, |store, cx| store.set_active_workspace_by_path(&path, cx));
    }

    fn current_workspace_entity(&self, cx: &App) -> Option<Entity<Workspace>> {
        self.multi_workspace
            .upgrade()
            .map(|multi_workspace| multi_workspace.read(cx).workspace().clone())
    }

    fn expand_workspace_section(
        &mut self,
        _: &ExpandWorkspaceSection,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.store.read(cx).active_project_id().map(str::to_owned) else {
            return;
        };
        self.store.update(cx, |store, cx| {
            store.set_project_collapsed(&project_id, false, cx);
        });
    }

    fn collapse_workspace_section(
        &mut self,
        _: &CollapseWorkspaceSection,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.store.read(cx).active_project_id().map(str::to_owned) else {
            return;
        };
        self.store.update(cx, |store, cx| {
            store.set_project_collapsed(&project_id, true, cx);
        });
    }

    fn render_project(
        &self,
        project: &ProjectEntry,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_collapsed = project.collapsed;
        let workspaces = self
            .store
            .read(cx)
            .workspaces_for_project(&project.id)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();

        v_flex()
            .w_full()
            .mb_1()
            .child(
                ListItem::new(format!("project-{}", project.id))
                    .spacing(ui::ListItemSpacing::Sparse)
                    .rounded()
                    .start_slot(Icon::new(if is_collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    }))
                    .end_slot(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Label::new(project_workspace_label(workspaces.len()))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                IconButton::new(
                                    format!("project-new-{}", project.id),
                                    IconName::Plus,
                                )
                                .shape(ui::IconButtonShape::Square)
                                .icon_color(Color::Muted)
                                .on_click(cx.listener({
                                    let project_id = project.id.clone();
                                    move |this, _: &ClickEvent, window, cx| {
                                        this.store.update(cx, |store, cx| {
                                            store.set_active_workspace(
                                                store
                                                    .primary_workspace_for_project(&project_id)
                                                    .map(|workspace| workspace.id.clone()),
                                                cx,
                                            );
                                        });
                                        if let Some(workspace) = this.current_workspace_entity(cx) {
                                            run_new_workspace_from_store(workspace, window, cx);
                                        }
                                    }
                                })),
                            ),
                    )
                    .on_click(cx.listener({
                        let project_id = project.id.clone();
                        move |this, _: &ClickEvent, _, cx| {
                            let collapsed = this
                                .store
                                .read(cx)
                                .project(&project_id)
                                .map(|project| !project.collapsed)
                                .unwrap_or(false);
                            this.store.update(cx, |store, cx| {
                                store.set_project_collapsed(&project_id, collapsed, cx);
                            });
                        }
                    }))
                    .child(
                        v_flex()
                            .w_full()
                            .child(Label::new(project.name.clone()).size(LabelSize::Small))
                            .child(
                                Label::new(project.repo_root.display().to_string())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    ),
            )
            .when(!is_collapsed, |this| {
                this.children(
                    workspaces
                        .iter()
                        .map(|workspace| {
                            self.render_workspace_row(workspace, window, cx)
                                .into_any_element()
                        })
                        .collect::<Vec<_>>(),
                )
            })
    }

    fn render_workspace_row(
        &self,
        workspace: &WorkspaceEntry,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.store.read(cx).active_workspace_id() == Some(workspace.id.as_str());
        let session_status = self.store.read(cx).aggregate_status_for_workspace(&workspace.id);
        let detail = workspace_detail(workspace, session_status.clone());
        let workspace_for_open = workspace.clone();
        let workspace_for_launch = workspace.clone();
        let workspace_for_delete = workspace.clone();

        ListItem::new(format!("workspace-{}", workspace.id))
            .toggle_state(selected)
            .indent_level(1)
            .spacing(ui::ListItemSpacing::Sparse)
            .rounded()
            .start_slot(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(Indicator::dot().color(status_color(session_status.clone())))
                    .child(Icon::new(match workspace.kind {
                        WorkspaceKind::Primary => IconName::Folder,
                        WorkspaceKind::Worktree => IconName::GitBranch,
                    })),
            )
            .end_hover_slot(
                h_flex()
                    .gap_1()
                    .child(
                        IconButton::new(
                            format!("launch-{}", workspace.id),
                            IconName::PlayFilled,
                        )
                        .shape(ui::IconButtonShape::Square)
                        .icon_color(Color::Muted)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.store.update(cx, |store, cx| {
                                store.set_active_workspace(Some(workspace_for_launch.id.clone()), cx);
                            });
                            if let Some(current_workspace) = this.current_workspace_entity(cx) {
                                run_launch_agent_from_store(current_workspace, window, cx);
                            }
                        })),
                    )
                    .when(workspace.managed, |this| {
                        this.child(
                            IconButton::new(
                                format!("delete-{}", workspace.id),
                                IconName::Trash,
                            )
                            .shape(ui::IconButtonShape::Square)
                            .icon_color(Color::Muted)
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.set_active_workspace(Some(workspace_for_delete.id.clone()), cx);
                                });
                                if let Some(current_workspace) = this.current_workspace_entity(cx) {
                                    run_delete_workspace_from_store(current_workspace, window, cx);
                                }
                            })),
                        )
                    }),
            )
            .tooltip({
                let path = workspace.worktree_path.display().to_string();
                move |window, cx| ui::Tooltip::text(path.clone())(window, cx)
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.store.update(cx, |store, cx| {
                    store.record_workspace_opened(&workspace_for_open.id, cx);
                });
                this.refresh_workspace_metadata(workspace_for_open.clone(), window, cx);
                if let Some(current_workspace) = this.current_workspace_entity(cx) {
                    open_workspace_path(
                        current_workspace,
                        workspace_for_open.worktree_path.clone(),
                        window,
                        cx,
                    )
                    .detach_and_log_err(cx);
                }
            }))
            .child(
                v_flex()
                    .w_full()
                    .child(Label::new(workspace.name.clone()).size(LabelSize::Small))
                    .child(
                        Label::new(detail)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
    }

    fn refresh_workspace_metadata(
        &self,
        workspace: WorkspaceEntry,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        cx.spawn_in(window, async move |_, cx| {
            let refresh = cx
                .background_spawn(async move {
                    superzed_git::refresh_workspace_path(&workspace.worktree_path)
                })
                .await;

            if let Ok(refresh) = refresh {
                store.update(cx, |store, cx| {
                    store.refresh_workspace_metadata(
                        &workspace.id,
                        Some(refresh.branch),
                        refresh.git_summary,
                        cx,
                    );
                });
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }
}

impl EventEmitter<SidebarEvent> for SuperzedSidebar {}

impl Focusable for SuperzedSidebar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SuperzedSidebar {
    fn render(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let projects = self.store.read(cx).projects().to_vec();
        let project_content = if projects.is_empty() {
            vec![
                v_flex()
                    .gap_1()
                    .py_4()
                    .child(Label::new("No repositories yet"))
                    .child(
                        Label::new("Add a local git repository to manage workspaces.")
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
            ]
        } else {
            projects
                .iter()
                .map(|project| self.render_project(project, window, cx).into_any_element())
                .collect::<Vec<_>>()
        };

        v_flex()
            .id("workspace-sidebar")
            .key_context("WorkspaceSidebar")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::expand_workspace_section))
            .on_action(cx.listener(Self::collapse_workspace_section))
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .child(
                v_flex()
                    .h_full()
                    .child(
                        h_flex()
                            .border_b_1()
                            .border_color(cx.theme().colors().border)
                            .px_2()
                            .h(px(40.))
                            .items_center()
                            .child(
                                Label::new("Workspaces")
                                    .size(LabelSize::Small)
                                    .color(Color::Default),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("superzed-sidebar-new-workspace", "New")
                                    .style(ui::ButtonStyle::Subtle)
                                    .icon(IconName::Plus)
                                    .label_size(LabelSize::Small)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        if let Some(current_workspace) =
                                            this.current_workspace_entity(cx)
                                        {
                                            run_new_workspace_from_store(
                                                current_workspace,
                                                window,
                                                cx,
                                            );
                                        }
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .px_2()
                            .py_2()
                            .children(project_content),
                    )
                    .child(
                        v_flex()
                            .border_t_1()
                            .border_color(cx.theme().colors().border)
                            .px_2()
                            .py_2()
                            .child(
                                Button::new("superzed-sidebar-add-project", "Add Repository")
                                    .full_width()
                                    .style(ui::ButtonStyle::Subtle)
                                    .icon(IconName::FolderOpen)
                                    .label_size(LabelSize::Small)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        if let Some(current_workspace) =
                                            this.current_workspace_entity(cx)
                                        {
                                            run_add_project_from_store(
                                                current_workspace,
                                                window,
                                                cx,
                                            );
                                        }
                                    })),
                            ),
                    ),
            )
    }
}

impl WorkspaceSidebar for SuperzedSidebar {
    fn width(&self, _: &App) -> Pixels {
        self.width.unwrap_or_else(|| px(300.))
    }

    fn set_width(&mut self, width: Option<Pixels>, cx: &mut Context<Self>) {
        self.width = width;
        cx.notify();
    }

    fn has_notifications(&self, cx: &App) -> bool {
        self.store.read(cx).workspaces().iter().any(|workspace| {
            matches!(
                self.store.read(cx).aggregate_status_for_workspace(&workspace.id),
                TaskStatus::NeedsAttention | TaskStatus::Failed
            )
        })
    }
}

pub struct SuperzedRightSidebar {
    project_panel: Entity<ProjectPanel>,
    git_panel: Entity<GitPanel>,
    store: Entity<SuperzedStore>,
    focus_handle: FocusHandle,
    width: Option<Pixels>,
    _active: bool,
    tab: RightSidebarTab,
    _subscriptions: Vec<Subscription>,
}

impl SuperzedRightSidebar {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        project_panel: Entity<ProjectPanel>,
        git_panel: Entity<GitPanel>,
        mut cx: AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| Self::new(workspace, project_panel, git_panel, window, cx))
        })
    }

    fn new(
        _workspace: &Workspace,
        project_panel: Entity<ProjectPanel>,
        git_panel: Entity<GitPanel>,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = SuperzedStore::global(cx);
        Self {
            project_panel,
            git_panel,
            store: store.clone(),
            focus_handle: cx.focus_handle(),
            width: None,
            _active: false,
            tab: RightSidebarTab::Changes,
            _subscriptions: vec![cx.observe(&store, |_, _, cx| cx.notify())],
        }
    }

    fn set_active_tab(&mut self, tab: RightSidebarTab, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for SuperzedRightSidebar {}

impl Focusable for SuperzedRightSidebar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SuperzedRightSidebar {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_workspace = self.store.read(cx).active_workspace().cloned();
        let title = active_workspace
            .as_ref()
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| "Workspace".into());
        let subtitle = active_workspace
            .as_ref()
            .map(|workspace| {
                format!(
                    "{} · {}",
                    workspace_kind_label(workspace.kind.clone()),
                    workspace.branch
                )
            })
            .unwrap_or_else(|| "No workspace selected".into());

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .bg(cx.theme().colors().panel_background)
                    .child(
                        h_flex()
                            .h(px(36.))
                            .px_2()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("superzed-right-tab-changes", "Changes")
                                    .icon(IconName::GitBranchAlt)
                                    .label_size(LabelSize::Small)
                                    .style(if self.tab == RightSidebarTab::Changes {
                                        ui::ButtonStyle::Filled
                                    } else {
                                        ui::ButtonStyle::Subtle
                                    })
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.set_active_tab(RightSidebarTab::Changes, cx);
                                    })),
                            )
                            .child(
                                Button::new("superzed-right-tab-files", "Files")
                                    .icon(IconName::FileTree)
                                    .label_size(LabelSize::Small)
                                    .style(if self.tab == RightSidebarTab::Files {
                                        ui::ButtonStyle::Filled
                                    } else {
                                        ui::ButtonStyle::Subtle
                                    })
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.set_active_tab(RightSidebarTab::Files, cx);
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                IconButton::new("superzed-right-close", IconName::Close)
                                    .shape(ui::IconButtonShape::Square)
                                    .tooltip(|window, cx| {
                                        ui::Tooltip::text("Close details sidebar")(window, cx)
                                    })
                                    .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                                        window.dispatch_action(Box::new(ToggleRightSidebar), cx);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .px_2()
                            .pb_2()
                            .gap_0p5()
                            .child(Label::new(title).size(LabelSize::Small))
                            .child(
                                Label::new(subtitle)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    ),
            )
            .child(
                div()
                    .size_full()
                    .child(match self.tab {
                        RightSidebarTab::Changes => self.git_panel.clone().into_any_element(),
                        RightSidebarTab::Files => self.project_panel.clone().into_any_element(),
                    }),
            )
    }
}

impl Panel for SuperzedRightSidebar {
    fn persistent_name() -> &'static str {
        "Superzed Right Sidebar"
    }

    fn panel_key() -> &'static str {
        "SuperzedRightSidebar"
    }

    fn position(&self, _: &gpui::Window, _: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        position == DockPosition::Right
    }

    fn set_position(
        &mut self,
        _: DockPosition,
        _: &mut gpui::Window,
        _: &mut Context<Self>,
    ) {
    }

    fn size(&self, _: &gpui::Window, _: &App) -> Pixels {
        self.width.unwrap_or_else(|| px(320.))
    }

    fn set_size(
        &mut self,
        size: Option<Pixels>,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.width = size;
        cx.notify();
    }

    fn icon(&self, _: &gpui::Window, _: &App) -> Option<IconName> {
        Some(IconName::FileTree)
    }

    fn icon_tooltip(&self, _: &gpui::Window, _: &App) -> Option<&'static str> {
        Some("Workspace Details")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleRightSidebar)
    }

    fn starts_open(&self, _: &gpui::Window, _: &App) -> bool {
        true
    }

    fn set_active(
        &mut self,
        active: bool,
        _: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self._active = active;
        cx.notify();
    }

    fn activation_priority(&self) -> u32 {
        10
    }
}

fn run_add_project(workspace: &mut Workspace, window: &mut gpui::Window, cx: &mut Context<Workspace>) {
    let store = SuperzedStore::global(cx);
    let workspace_handle = cx.entity();
    let app_state = workspace.app_state().clone();
    let prompt = workspace.prompt_for_open_path(
        PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add Project".into()),
        },
        DirectoryLister::Local(workspace.project().clone(), app_state.fs.clone()),
        window,
        cx,
    );
    let default_preset_id = store.read(cx).default_preset().id.clone();

    cx.spawn_in(window, async move |_, cx| {
        let Some(paths) = prompt.await.log_err().flatten() else {
            return anyhow::Ok(());
        };
        let Some(path) = paths.into_iter().next() else {
            return anyhow::Ok(());
        };

        let registration = cx
            .background_spawn(async move { superzed_git::register_project(&path, &default_preset_id) })
            .await;

        workspace_handle
            .update_in(cx, |workspace, window, cx| match registration {
                Ok(registration) => {
                    let existing_primary = store
                        .read(cx)
                        .project_for_repo_root(&registration.project.repo_root)
                        .and_then(|project| {
                            store
                                .read(cx)
                                .primary_workspace_for_project(&project.id)
                                .cloned()
                        });

                    let primary_workspace = if let Some(existing) = existing_primary {
                        existing
                    } else {
                        store.update(cx, |store, cx| {
                            store.upsert_project_bundle(
                                registration.project.clone(),
                                registration.primary_workspace.clone(),
                                cx,
                            );
                        });
                        registration.primary_workspace
                    };

                    store.update(cx, |store, cx| {
                        store.record_workspace_opened(&primary_workspace.id, cx);
                    });
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::unique::<SuperzedSidebar>(),
                            format!("Added {}", primary_workspace.name),
                        ),
                        cx,
                    );
                    open_workspace_path(
                        workspace_handle.clone(),
                        primary_workspace.worktree_path.clone(),
                        window,
                        cx,
                    )
                    .detach_and_log_err(cx);
                }
                Err(error) => workspace.show_toast(
                    Toast::new(
                        NotificationId::unique::<SuperzedSidebar>(),
                        format!("Failed to add project: {error}"),
                    ),
                    cx,
                ),
            })
            .ok();

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

fn run_new_workspace(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    let store = SuperzedStore::global(cx);
    let workspace_handle = cx.entity();
    let Some(project) = store
        .read(cx)
        .active_project()
        .cloned()
        .or_else(|| store.read(cx).projects().first().cloned())
    else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Add a project before creating a workspace.",
            ),
            cx,
        );
        return;
    };

    let preset_id = store.read(cx).default_preset().id.clone();
    cx.spawn_in(window, async move |_, cx| {
        let outcome = cx
            .background_spawn(async move { superzed_git::create_workspace(&project, &preset_id) })
            .await;

        workspace_handle
            .update_in(cx, |workspace, window, cx| match outcome {
                Ok(outcome) => {
                    let workspace_entry = outcome.workspace.clone();
                    store.update(cx, |store, cx| {
                        store.upsert_workspace(workspace_entry.clone(), cx);
                        store.record_workspace_opened(&workspace_entry.id, cx);
                    });
                    let message = outcome.warning.map_or_else(
                        || format!("Created {}", workspace_entry.name),
                        |warning| format!("Created {} with warnings: {warning}", workspace_entry.name),
                    );
                    workspace.show_toast(
                        Toast::new(NotificationId::unique::<SuperzedSidebar>(), message),
                        cx,
                    );
                    open_workspace_path(
                        workspace_handle.clone(),
                        workspace_entry.worktree_path.clone(),
                        window,
                        cx,
                    )
                    .detach_and_log_err(cx);
                }
                Err(error) => workspace.show_toast(
                    Toast::new(
                        NotificationId::unique::<SuperzedSidebar>(),
                        format!("Failed to create workspace: {error}"),
                    ),
                    cx,
                ),
            })
            .ok();

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

fn run_new_workspace_from_store(
    workspace_handle: Entity<Workspace>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    workspace_handle.update(cx, |workspace, cx| {
        run_new_workspace(workspace, window, cx);
    });
}

fn run_launch_agent(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    let workspace_handle = cx.entity();
    let store = SuperzedStore::global(cx);
    let Some(workspace_entry) = store
        .read(cx)
        .active_workspace()
        .cloned()
        .or_else(|| store.read(cx).workspaces().first().cloned())
    else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Select a workspace first.",
            ),
            cx,
        );
        return;
    };
    let Some(preset) = store
        .read(cx)
        .preset(&workspace_entry.agent_preset_id)
        .cloned()
        .or_else(|| store.read(cx).presets().first().cloned())
    else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "No agent presets are configured.",
            ),
            cx,
        );
        return;
    };

    let store_for_session = store.clone();
    let session = store.update(cx, |store, cx| {
        let label = format!("{} · {}", workspace_entry.name, preset.label);
        store.start_session(&workspace_entry.id, &preset, label, cx)
    });

    let target_path = workspace_entry.worktree_path.clone();
    let switch_task = open_workspace_path(workspace_handle.clone(), target_path, window, cx);
    let maybe_multi_workspace = window.window_handle().downcast::<MultiWorkspace>();

    cx.spawn_in(window, async move |_, cx| {
        let switch_result = switch_task.await;
        if let Err(error) = switch_result {
            workspace_handle
                .update_in(cx, |workspace, _, cx| {
                    store_for_session.update(cx, |store, cx| {
                        store.update_session_status(
                            &session.id,
                            TaskStatus::Failed,
                            Some(error.to_string()),
                            cx,
                        );
                    });
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::unique::<SuperzedSidebar>(),
                            format!("Failed to open workspace: {error}"),
                        ),
                        cx,
                    );
                })
                .ok();
            return anyhow::Ok(());
        }

        let active_workspace = if let Some(multi_workspace) = maybe_multi_workspace {
            multi_workspace.update(cx, |multi_workspace, _, _| multi_workspace.workspace().clone())?
        } else {
            workspace_handle.clone()
        };

        let spawn = superzed_agent::spawn_for_workspace(&workspace_entry, &session, &preset);
        let launch = active_workspace.update_in(cx, |workspace, window, cx| {
            workspace.spawn_in_terminal(spawn, window, cx)
        })?;

        let result = launch.await;
        let (status, reason) = match result {
            Some(Ok(exit_status)) if exit_status.success() => (TaskStatus::Completed, None),
            Some(Ok(exit_status)) => (
                TaskStatus::NeedsAttention,
                Some(format!("Agent exited with status {:?}", exit_status.code())),
            ),
            Some(Err(error)) => (TaskStatus::Failed, Some(error.to_string())),
            None => (
                TaskStatus::NeedsAttention,
                Some("Agent launch was cancelled".into()),
            ),
        };

        store_for_session.update(cx, |store, cx| {
            store.update_session_status(&session.id, status, reason, cx);
        });

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

fn run_launch_agent_from_store(
    workspace_handle: Entity<Workspace>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    workspace_handle.update(cx, |workspace, cx| {
        run_launch_agent(workspace, window, cx);
    });
}

fn run_reveal_changes(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    let workspace_handle = cx.entity();
    let store = SuperzedStore::global(cx);
    let Some(workspace_entry) = store
        .read(cx)
        .active_workspace()
        .cloned()
        .or_else(|| store.read(cx).workspaces().first().cloned())
    else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Select a workspace first.",
            ),
            cx,
        );
        return;
    };
    let target_path = workspace_entry.worktree_path.clone();
    let switch_task = open_workspace_path(workspace_handle.clone(), target_path, window, cx);
    let maybe_multi_workspace = window.window_handle().downcast::<MultiWorkspace>();

    cx.spawn_in(window, async move |_, cx| {
        if let Err(error) = switch_task.await {
            workspace_handle
                .update_in(cx, |workspace, _, cx| {
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::unique::<SuperzedSidebar>(),
                            format!("Failed to open workspace: {error}"),
                        ),
                        cx,
                    );
                })
                .ok();
            return anyhow::Ok(());
        }

        let active_workspace = if let Some(multi_workspace) = maybe_multi_workspace {
            multi_workspace.update(cx, |multi_workspace, _, _| multi_workspace.workspace().clone())?
        } else {
            workspace_handle.clone()
        };

        active_workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.open_panel::<SuperzedRightSidebar>(window, cx);
                workspace.focus_panel::<SuperzedRightSidebar>(window, cx);
                if let Some(panel) = workspace.panel::<SuperzedRightSidebar>(cx) {
                    panel.update(cx, |panel, cx| panel.set_active_tab(RightSidebarTab::Changes, cx));
                }
            })
            .ok();

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

fn run_open_workspace_in_new_window(
    workspace: &mut Workspace,
    _window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    let store = SuperzedStore::global(cx);
    let Some(workspace_entry) = store
        .read(cx)
        .active_workspace()
        .cloned()
        .or_else(|| store.read(cx).workspaces().first().cloned())
    else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Select a workspace first.",
            ),
            cx,
        );
        return;
    };

    let app_state = workspace.app_state().clone();
    let paths = vec![workspace_entry.worktree_path.clone()];
    cx.spawn(async move |_, cx| {
        cx.update(|cx| {
            workspace::open_paths(
                &paths,
                app_state,
                OpenOptions {
                    open_new_workspace: Some(true),
                    focus: Some(true),
                    visible: Some(OpenVisible::All),
                    ..Default::default()
                },
                cx,
            )
        })
        .await?;
        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

fn run_delete_workspace(
    workspace: &mut Workspace,
    window: &mut gpui::Window,
    cx: &mut Context<Workspace>,
) {
    let workspace_handle = cx.entity();
    let store = SuperzedStore::global(cx);
    let Some(workspace_entry) = store
        .read(cx)
        .active_workspace()
        .cloned()
        .or_else(|| store.read(cx).workspaces().first().cloned())
    else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Select a workspace first.",
            ),
            cx,
        );
        return;
    };
    if workspace_entry.kind == WorkspaceKind::Primary || !workspace_entry.managed {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Primary workspaces cannot be deleted.",
            ),
            cx,
        );
        return;
    }
    let Some(project) = store.read(cx).project(&workspace_entry.project_id).cloned() else {
        workspace.show_toast(
            Toast::new(
                NotificationId::unique::<SuperzedSidebar>(),
                "Missing project metadata.",
            ),
            cx,
        );
        return;
    };

    let prompt = window.prompt(
        PromptLevel::Warning,
        "Delete workspace?",
        Some(&format!(
            "Delete `{}` and remove its worktree at {}?",
            workspace_entry.name,
            workspace_entry.worktree_path.display()
        )),
        &["Cancel", "Delete"],
        cx,
    );

    cx.spawn_in(window, async move |this, cx| {
        if prompt.await != Ok(1) {
            return anyhow::Ok(());
        }

        let workspace_to_delete = workspace_entry.clone();
        let delete_result = cx
            .background_spawn(async move {
                superzed_git::delete_workspace(&workspace_to_delete, &project.repo_root, false)
            })
            .await;

        this
            .update_in(cx, |workspace, window, cx| match delete_result {
                Ok(()) => {
                    store.update(cx, |store, cx| {
                        store.remove_workspace(&workspace_entry.id, cx);
                    });
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::unique::<SuperzedSidebar>(),
                            format!("Deleted {}", workspace_entry.name),
                        ),
                        cx,
                    );
                    if let Some(primary_workspace) = store
                        .read(cx)
                        .primary_workspace_for_project(&project.id)
                        .cloned()
                    {
                        store.update(cx, |store, cx| {
                            store.record_workspace_opened(&primary_workspace.id, cx);
                        });
                        open_workspace_path(
                            workspace_handle.clone(),
                            primary_workspace.worktree_path.clone(),
                            window,
                            cx,
                        )
                        .detach_and_log_err(cx);
                    }
                }
                Err(error) => {
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::unique::<SuperzedSidebar>(),
                            format!("Failed to remove workspace: {error}"),
                        ),
                        cx,
                    );
                }
            })
            .ok();

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

fn run_delete_workspace_from_store(
    workspace_handle: Entity<Workspace>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    workspace_handle.update(cx, |workspace, cx| {
        run_delete_workspace(workspace, window, cx);
    });
}

fn run_add_project_from_store(
    workspace_handle: Entity<Workspace>,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    workspace_handle.update(cx, |workspace, cx| {
        run_add_project(workspace, window, cx);
    });
}

fn open_workspace_path(
    workspace_handle: Entity<Workspace>,
    path: std::path::PathBuf,
    window: &mut gpui::Window,
    cx: &mut App,
) -> Task<anyhow::Result<()>> {
    workspace_handle.update(cx, |workspace, cx| {
        workspace.open_workspace_for_paths(false, vec![path], window, cx)
    })
}

fn workspace_root_path(workspace: &Entity<Workspace>, cx: &App) -> Option<std::path::PathBuf> {
    let project = workspace.read(cx).project();
    project.read(cx).visible_worktrees(cx).find_map(|worktree| {
        worktree
            .read(cx)
            .as_local()
            .map(|local| local.abs_path().to_path_buf())
    })
}

fn workspace_detail(workspace: &WorkspaceEntry, status: TaskStatus) -> String {
    let mut parts = vec![
        workspace_kind_label(workspace.kind.clone()).to_string(),
        workspace.branch.clone(),
        status_label(status).to_string(),
    ];
    if let Some(summary) = &workspace.git_summary {
        let mut counts = Vec::new();
        if summary.changed_files > 0 {
            counts.push(format!("{} changed", summary.changed_files));
        }
        if summary.staged_files > 0 {
            counts.push(format!("{} staged", summary.staged_files));
        }
        if summary.untracked_files > 0 {
            counts.push(format!("{} untracked", summary.untracked_files));
        }
        if !counts.is_empty() {
            parts.push(counts.join(" · "));
        }
    }
    parts.join(" · ")
}

fn project_workspace_label(count: usize) -> String {
    match count {
        1 => "1 workspace".to_string(),
        _ => format!("{count} workspaces"),
    }
}

fn workspace_kind_label(kind: WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Primary => "Main repo",
        WorkspaceKind::Worktree => "Git worktree",
    }
}

fn status_color(status: TaskStatus) -> Color {
    match status {
        TaskStatus::Idle => Color::Muted,
        TaskStatus::Starting => Color::Accent,
        TaskStatus::Running => Color::Success,
        TaskStatus::NeedsAttention => Color::Warning,
        TaskStatus::Completed => Color::Success,
        TaskStatus::Failed => Color::Error,
    }
}

fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Idle => "Idle",
        TaskStatus::Starting => "Starting",
        TaskStatus::Running => "Running",
        TaskStatus::NeedsAttention => "Attention",
        TaskStatus::Completed => "Completed",
        TaskStatus::Failed => "Failed",
    }
}
