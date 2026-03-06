use chrono::{DateTime, Utc};
use gpui::{App, AppContext, Context, Entity, Global};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Idle,
    Starting,
    Running,
    NeedsAttention,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    #[default]
    Primary,
    Worktree,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitChangeSummary {
    pub changed_files: usize,
    pub staged_files: usize,
    pub untracked_files: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPreset {
    pub id: String,
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub attention_patterns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub repo_root: PathBuf,
    #[serde(default)]
    pub collapsed: bool,
    pub created_at: DateTime<Utc>,
    pub last_opened_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub id: String,
    pub project_id: String,
    pub kind: WorkspaceKind,
    pub name: String,
    pub branch: String,
    pub worktree_path: PathBuf,
    pub agent_preset_id: String,
    pub managed: bool,
    #[serde(default)]
    pub git_summary: Option<GitChangeSummary>,
    #[serde(default)]
    pub last_attention_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_opened_at: DateTime<Utc>,
}

impl WorkspaceEntry {
    pub fn is_existing_path(&self) -> bool {
        self.worktree_path.exists()
    }

    pub fn is_primary(&self) -> bool {
        self.kind == WorkspaceKind::Primary
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSession {
    pub id: String,
    pub workspace_id: String,
    pub preset_id: String,
    pub label: String,
    pub status: TaskStatus,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub exited_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_attention_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuperzedState {
    pub active_project_id: Option<String>,
    pub active_workspace_id: Option<String>,
    pub projects: Vec<ProjectEntry>,
    pub workspaces: Vec<WorkspaceEntry>,
    pub sessions: Vec<AgentSession>,
    pub presets: Vec<AgentPreset>,
}

impl Default for SuperzedState {
    fn default() -> Self {
        Self {
            active_project_id: None,
            active_workspace_id: None,
            projects: Vec::new(),
            workspaces: Vec::new(),
            sessions: Vec::new(),
            presets: default_presets(),
        }
    }
}

pub struct SuperzedStore {
    state_path: PathBuf,
    state: SuperzedState,
}

struct GlobalSuperzedStore(Entity<SuperzedStore>);

impl Global for GlobalSuperzedStore {}

impl SuperzedStore {
    pub fn init(cx: &mut App) {
        let store = cx.new(|_| Self::load());
        cx.set_global(GlobalSuperzedStore(store));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalSuperzedStore>().0.clone()
    }

    pub fn projects(&self) -> &[ProjectEntry] {
        &self.state.projects
    }

    pub fn workspaces(&self) -> &[WorkspaceEntry] {
        &self.state.workspaces
    }

    pub fn sessions(&self) -> &[AgentSession] {
        &self.state.sessions
    }

    pub fn active_project_id(&self) -> Option<&str> {
        self.state.active_project_id.as_deref()
    }

    pub fn active_workspace_id(&self) -> Option<&str> {
        self.state.active_workspace_id.as_deref()
    }

    pub fn active_project(&self) -> Option<&ProjectEntry> {
        self.state
            .active_project_id
            .as_deref()
            .and_then(|id| self.project(id))
    }

    pub fn active_workspace(&self) -> Option<&WorkspaceEntry> {
        self.state
            .active_workspace_id
            .as_deref()
            .and_then(|id| self.workspace(id))
    }

    pub fn project(&self, id: &str) -> Option<&ProjectEntry> {
        self.state.projects.iter().find(|project| project.id == id)
    }

    pub fn workspace(&self, id: &str) -> Option<&WorkspaceEntry> {
        self.state
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
    }

    pub fn workspace_for_path(&self, path: &Path) -> Option<&WorkspaceEntry> {
        self.state
            .workspaces
            .iter()
            .find(|workspace| workspace.worktree_path == path)
    }

    pub fn project_for_workspace(&self, workspace_id: &str) -> Option<&ProjectEntry> {
        self.workspace(workspace_id)
            .and_then(|workspace| self.project(&workspace.project_id))
    }

    pub fn project_for_repo_root(&self, repo_root: &Path) -> Option<&ProjectEntry> {
        self.state
            .projects
            .iter()
            .find(|project| project.repo_root == repo_root)
    }

    pub fn primary_workspace_for_project(&self, project_id: &str) -> Option<&WorkspaceEntry> {
        self.state
            .workspaces
            .iter()
            .find(|workspace| workspace.project_id == project_id && workspace.is_primary())
    }

    pub fn workspaces_for_project(&self, project_id: &str) -> Vec<&WorkspaceEntry> {
        let mut workspaces = self
            .state
            .workspaces
            .iter()
            .filter(|workspace| workspace.project_id == project_id)
            .collect::<Vec<_>>();
        workspaces.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| right.last_opened_at.cmp(&left.last_opened_at))
                .then_with(|| left.name.cmp(&right.name))
        });
        workspaces
    }

    pub fn presets(&self) -> &[AgentPreset] {
        &self.state.presets
    }

    pub fn default_preset(&self) -> &AgentPreset {
        self.state
            .presets
            .first()
            .expect("Superzed requires at least one agent preset")
    }

    pub fn preset(&self, id: &str) -> Option<&AgentPreset> {
        self.state.presets.iter().find(|preset| preset.id == id)
    }

    pub fn set_active_workspace(
        &mut self,
        workspace_id: Option<impl Into<String>>,
        cx: &mut Context<Self>,
    ) {
        self.state.active_workspace_id = workspace_id.map(Into::into);
        self.state.active_project_id = self
            .state
            .active_workspace_id
            .as_deref()
            .and_then(|workspace_id| self.workspace(workspace_id))
            .map(|workspace| workspace.project_id.clone())
            .or_else(|| self.state.projects.first().map(|project| project.id.clone()));
        self.persist_and_notify(cx);
    }

    pub fn set_active_workspace_by_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        let workspace_id = self.workspace_for_path(path).map(|workspace| workspace.id.clone());
        self.set_active_workspace(workspace_id, cx);
    }

    pub fn set_project_collapsed(
        &mut self,
        project_id: &str,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = self
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == project_id)
        {
            project.collapsed = collapsed;
            self.persist_and_notify(cx);
        }
    }

    pub fn upsert_project(&mut self, project: ProjectEntry, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .state
            .projects
            .iter_mut()
            .find(|existing| existing.id == project.id)
        {
            *existing = project;
        } else {
            self.state.projects.push(project);
        }
        self.normalize();
        self.persist_and_notify(cx);
    }

    pub fn upsert_workspace(&mut self, workspace: WorkspaceEntry, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .state
            .workspaces
            .iter_mut()
            .find(|existing| existing.id == workspace.id)
        {
            *existing = workspace;
        } else {
            self.state.workspaces.push(workspace);
        }
        self.normalize();
        self.persist_and_notify(cx);
    }

    pub fn upsert_project_bundle(
        &mut self,
        project: ProjectEntry,
        workspace: WorkspaceEntry,
        cx: &mut Context<Self>,
    ) {
        let project_id = project.id.clone();
        let workspace_id = workspace.id.clone();
        if self.project(&project.id).is_none() {
            self.state.projects.push(project);
        } else {
            self.state
                .projects
                .iter_mut()
                .filter(|existing| existing.id == project.id)
                .for_each(|existing| *existing = project.clone());
        }

        if self.workspace(&workspace.id).is_none() {
            self.state.workspaces.push(workspace);
        } else {
            self.state
                .workspaces
                .iter_mut()
                .filter(|existing| existing.id == workspace.id)
                .for_each(|existing| *existing = workspace.clone());
        }

        self.normalize();
        if self.state.active_workspace_id.is_none() {
            self.state.active_workspace_id = Some(workspace_id);
            self.state.active_project_id = Some(project_id);
        }
        self.persist_and_notify(cx);
    }

    pub fn record_workspace_opened(&mut self, workspace_id: &str, cx: &mut Context<Self>) {
        let now = Utc::now();
        let Some(workspace) = self
            .state
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return;
        };

        workspace.last_opened_at = now;
        let project_id = workspace.project_id.clone();
        if let Some(project) = self
            .state
            .projects
            .iter_mut()
            .find(|project| project.id == project_id)
        {
            project.last_opened_at = now;
        }

        self.state.active_workspace_id = Some(workspace_id.to_string());
        self.state.active_project_id = Some(project_id);
        self.persist_and_notify(cx);
    }

    pub fn refresh_workspace_metadata(
        &mut self,
        workspace_id: &str,
        branch: Option<String>,
        git_summary: Option<GitChangeSummary>,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self
            .state
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        {
            if let Some(branch) = branch {
                workspace.branch = branch;
            }
            workspace.git_summary = git_summary;
            self.persist_and_notify(cx);
        }
    }

    pub fn remove_workspace(
        &mut self,
        workspace_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<WorkspaceEntry> {
        let ix = self
            .state
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)?;
        let removed = self.state.workspaces.remove(ix);
        self.state
            .sessions
            .retain(|session| session.workspace_id != removed.id);
        self.normalize();
        self.persist_and_notify(cx);
        Some(removed)
    }

    pub fn start_session(
        &mut self,
        workspace_id: &str,
        preset: &AgentPreset,
        label: String,
        cx: &mut Context<Self>,
    ) -> AgentSession {
        let session = AgentSession {
            id: Uuid::new_v4().to_string(),
            workspace_id: workspace_id.to_string(),
            preset_id: preset.id.clone(),
            label,
            status: TaskStatus::Starting,
            started_at: Utc::now(),
            exited_at: None,
            last_attention_reason: None,
        };
        self.state.sessions.push(session.clone());
        self.persist_and_notify(cx);
        session
    }

    pub fn update_session_status(
        &mut self,
        session_id: &str,
        status: TaskStatus,
        reason: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self
            .state
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            let finished = matches!(
                status,
                TaskStatus::Idle
                    | TaskStatus::NeedsAttention
                    | TaskStatus::Completed
                    | TaskStatus::Failed
            );
            session.status = status.clone();
            session.last_attention_reason = reason.clone();
            if finished {
                session.exited_at = Some(Utc::now());
            }

            if let Some(workspace) = self
                .state
                .workspaces
                .iter_mut()
                .find(|workspace| workspace.id == session.workspace_id)
            {
                workspace.last_attention_reason = reason;
                workspace.last_opened_at = Utc::now();
            }

            self.persist_and_notify(cx);
        }
    }

    pub fn latest_session_for_workspace(&self, workspace_id: &str) -> Option<&AgentSession> {
        self.state
            .sessions
            .iter()
            .filter(|session| session.workspace_id == workspace_id)
            .max_by(|left, right| left.started_at.cmp(&right.started_at))
    }

    pub fn aggregate_status_for_workspace(&self, workspace_id: &str) -> TaskStatus {
        self.latest_session_for_workspace(workspace_id)
            .map(|session| session.status.clone())
            .unwrap_or(TaskStatus::Idle)
    }

    fn load() -> Self {
        let state_path = state_path();
        let mut state = fs::read_to_string(&state_path)
            .ok()
            .and_then(|contents| serde_json::from_str::<SuperzedState>(&contents).ok())
            .or_else(load_legacy_state)
            .unwrap_or_default();

        state.projects.retain(|project| project.repo_root.exists());
        state.workspaces.retain(WorkspaceEntry::is_existing_path);

        let workspace_ids = state
            .workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<BTreeSet<_>>();
        state
            .sessions
            .retain(|session| workspace_ids.contains(&session.workspace_id));

        if state.presets.is_empty() {
            state.presets = default_presets();
        }

        let mut store = Self { state_path, state };
        store.normalize();
        store
    }

    fn normalize(&mut self) {
        self.state
            .projects
            .sort_by(|left, right| right.last_opened_at.cmp(&left.last_opened_at));
        self.state.workspaces.sort_by(|left, right| {
            left.project_id
                .cmp(&right.project_id)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| right.last_opened_at.cmp(&left.last_opened_at))
                .then_with(|| left.name.cmp(&right.name))
        });

        let project_ids = self
            .state
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect::<BTreeSet<_>>();
        self.state
            .workspaces
            .retain(|workspace| project_ids.contains(&workspace.project_id));

        let existing_project_ids = self
            .state
            .projects
            .iter()
            .map(|project| project.id.as_str())
            .collect::<BTreeSet<_>>();
        if self
            .state
            .active_project_id
            .as_deref()
            .is_some_and(|id| !existing_project_ids.contains(id))
        {
            self.state.active_project_id = self.state.projects.first().map(|project| project.id.clone());
        }

        let existing_workspace_ids = self
            .state
            .workspaces
            .iter()
            .map(|workspace| workspace.id.as_str())
            .collect::<BTreeSet<_>>();
        if self
            .state
            .active_workspace_id
            .as_deref()
            .is_some_and(|id| !existing_workspace_ids.contains(id))
        {
            self.state.active_workspace_id = self.state.workspaces.first().map(|workspace| workspace.id.clone());
        }

        if self.state.active_project_id.is_none() {
            self.state.active_project_id = self.state.projects.first().map(|project| project.id.clone());
        }
        if self.state.active_workspace_id.is_none() {
            self.state.active_workspace_id = self.state.workspaces.first().map(|workspace| workspace.id.clone());
        }
    }

    fn persist_and_notify(&self, cx: &mut Context<Self>) {
        if let Err(error) = self.persist() {
            log::error!("failed to persist Superzed state: {error:#}");
        }
        cx.notify();
    }

    fn persist(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.state_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(&self.state)?;
        fs::write(&self.state_path, contents)?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct LegacyState {
    active_task_id: Option<String>,
    #[serde(default)]
    tasks: Vec<LegacyTaskWorkspace>,
    #[serde(default)]
    presets: Vec<AgentPreset>,
}

#[derive(Clone, Deserialize)]
struct LegacyTaskWorkspace {
    id: String,
    name: String,
    repo_root: PathBuf,
    worktree_path: PathBuf,
    branch: String,
    agent_preset_id: String,
    status: TaskStatus,
    managed: bool,
    #[serde(default)]
    last_attention_reason: Option<String>,
    created_at: DateTime<Utc>,
    last_event_at: DateTime<Utc>,
}

fn load_legacy_state() -> Option<SuperzedState> {
    let legacy_path = legacy_state_path();
    let contents = fs::read_to_string(&legacy_path).ok()?;
    let legacy = serde_json::from_str::<LegacyState>(&contents).ok()?;

    let presets = if legacy.presets.is_empty() {
        default_presets()
    } else {
        legacy.presets
    };
    let default_preset_id = presets
        .first()
        .map(|preset| preset.id.clone())
        .unwrap_or_else(|| "codex".to_string());

    let mut state = SuperzedState {
        active_project_id: None,
        active_workspace_id: legacy.active_task_id.clone(),
        projects: Vec::new(),
        workspaces: Vec::new(),
        sessions: Vec::new(),
        presets,
    };

    for task in legacy.tasks {
        if !task.worktree_path.exists() {
            continue;
        }

        let repo_root = task.repo_root.clone();
        let repo_name = repo_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Project")
            .to_string();
        let project_id = state
            .projects
            .iter()
            .find(|project| project.repo_root == repo_root)
            .map(|project| project.id.clone())
            .unwrap_or_else(|| {
                let id = Uuid::new_v4().to_string();
                state.projects.push(ProjectEntry {
                    id: id.clone(),
                    name: repo_name.clone(),
                    repo_root: repo_root.clone(),
                    collapsed: false,
                    created_at: task.created_at,
                    last_opened_at: task.last_event_at,
                });
                id
            });

        if state
            .workspaces
            .iter()
            .all(|workspace| workspace.project_id != project_id || !workspace.is_primary())
        {
            state.workspaces.push(WorkspaceEntry {
                id: Uuid::new_v4().to_string(),
                project_id: project_id.clone(),
                kind: WorkspaceKind::Primary,
                name: repo_name.clone(),
                branch: "HEAD".to_string(),
                worktree_path: repo_root.clone(),
                agent_preset_id: default_preset_id.clone(),
                managed: false,
                git_summary: None,
                last_attention_reason: None,
                created_at: task.created_at,
                last_opened_at: task.last_event_at,
            });
        }

        state.workspaces.push(WorkspaceEntry {
            id: task.id.clone(),
            project_id: project_id.clone(),
            kind: WorkspaceKind::Worktree,
            name: task.name,
            branch: task.branch,
            worktree_path: task.worktree_path,
            agent_preset_id: task.agent_preset_id,
            managed: task.managed,
            git_summary: None,
            last_attention_reason: task.last_attention_reason.clone(),
            created_at: task.created_at,
            last_opened_at: task.last_event_at,
        });

        if matches!(
            task.status,
            TaskStatus::Running
                | TaskStatus::NeedsAttention
                | TaskStatus::Completed
                | TaskStatus::Failed
        ) {
            state.sessions.push(AgentSession {
                id: Uuid::new_v4().to_string(),
                workspace_id: task.id,
                preset_id: default_preset_id.clone(),
                label: "Migrated session".to_string(),
                status: task.status,
                started_at: task.created_at,
                exited_at: Some(task.last_event_at),
                last_attention_reason: task.last_attention_reason,
            });
        }
    }

    Some(state)
}

fn state_path() -> PathBuf {
    paths::data_dir().join("superzed").join("state.json")
}

fn legacy_state_path() -> PathBuf {
    paths::data_dir().join("superzed").join("tasks.json")
}

fn default_presets() -> Vec<AgentPreset> {
    vec![
        AgentPreset {
            id: "codex".into(),
            label: "Codex".into(),
            command: "codex".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            attention_patterns: vec!["needs attention".into(), "blocked".into()],
        },
        AgentPreset {
            id: "claude-code".into(),
            label: "Claude Code".into(),
            command: "claude".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            attention_patterns: vec!["waiting for input".into()],
        },
        AgentPreset {
            id: "gemini".into(),
            label: "Gemini CLI".into(),
            command: "gemini".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            attention_patterns: vec!["press enter".into()],
        },
    ]
}

pub fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}
