//! Shared server state.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use milim_agents::AgentStore;
use milim_automation::ScheduleStore;
use milim_core::config::ServerConfiguration;
use milim_identity::AccessKeyValidator;
use milim_inference::SharedService;
use milim_memory::MemoryStore;
use milim_skills::SkillStore;
use milim_tools::ToolRegistry;
use serde::Serialize;

use crate::companion::MobileCompanionBridge;
use crate::control::RunManager;
use crate::media_library::MediaLibrary;
use crate::preview_runtime::PreviewRuntimeManager;
use crate::threads::ThreadSupervisor;

#[derive(Clone)]
pub(crate) struct AccountRuntimeToolSession {
    pub token: String,
    pub registry: ToolRegistry,
    pub review: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScheduleRunEvent {
    pub id: String,
    pub schedule_id: String,
    pub schedule_name: String,
    pub prompt: String,
    pub response: String,
    pub model: String,
    pub ran_at: i64,
}

#[derive(Default)]
pub struct ScheduleRunQueue {
    events: Mutex<VecDeque<ScheduleRunEvent>>,
}

#[derive(Clone)]
pub(crate) struct McpAppViewDocument {
    pub html: String,
    pub csp: String,
}

struct CachedMcpAppView {
    document: McpAppViewDocument,
    expires_at: Instant,
}

#[derive(Default)]
pub(crate) struct McpAppViewStore {
    views: Mutex<HashMap<String, CachedMcpAppView>>,
}

impl McpAppViewStore {
    pub fn insert(&self, document: McpAppViewDocument) -> String {
        const MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
        let now = Instant::now();
        let mut views = self.views.lock().expect("MCP App view store poisoned");
        views.retain(|_, view| view.expires_at > now);
        while !views.is_empty()
            && views
                .values()
                .map(|view| view.document.html.len())
                .sum::<usize>()
                + document.html.len()
                > MAX_PENDING_BYTES
        {
            let oldest = views
                .iter()
                .min_by_key(|(_, view)| view.expires_at)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                views.remove(&id);
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        views.insert(
            id.clone(),
            CachedMcpAppView {
                document,
                expires_at: now + Duration::from_secs(10 * 60),
            },
        );
        id
    }

    pub fn take(&self, id: &str) -> Option<McpAppViewDocument> {
        let now = Instant::now();
        let mut views = self.views.lock().expect("MCP App view store poisoned");
        views.retain(|_, view| view.expires_at > now);
        views.remove(id).map(|view| view.document)
    }
}

impl ScheduleRunQueue {
    pub fn push(&self, event: ScheduleRunEvent) {
        let mut events = self.events.lock().expect("schedule run queue poisoned");
        events.push_back(event);
        while events.len() > 100 {
            events.pop_front();
        }
    }

    pub fn take(&self) -> Vec<ScheduleRunEvent> {
        let mut events = self.events.lock().expect("schedule run queue poisoned");
        events.drain(..).collect()
    }
}

/// Cloneable handle to everything request handlers need.
#[derive(Clone)]
pub struct AppState {
    /// The active inference backend.
    pub service: SharedService,
    /// Effective server configuration.
    pub config: Arc<ServerConfiguration>,
    /// Accepted API keys. Empty ⇒ auth disabled (dev/loopback).
    pub api_keys: Arc<HashSet<String>>,
    /// Bypass auth for loopback peers.
    pub trust_loopback: bool,
    /// Process start time (unix seconds), used as the model `created` field.
    pub created: u64,
    /// Optional msk-v1 access-key validator (secp256k1). When set, valid
    /// `msk-v1` bearer tokens are accepted in addition to static API keys.
    pub access_validator: Option<Arc<AccessKeyValidator>>,
    /// Tools exposed via `/mcp/tools` + `/mcp/call` (and, later, the agent loop).
    pub tools: Option<Arc<ToolRegistry>>,
    /// Optional memory/RAG store exposed via `/memory/ingest` + `/memory/search`.
    pub memory: Option<Arc<MemoryStore>>,
    /// Optional named-agent store exposed via `/agents` + `/agents/{id}/run`.
    pub agents: Option<Arc<AgentStore>>,
    /// Optional child-thread supervisor exposed via child orchestration tools.
    pub threads: Option<Arc<ThreadSupervisor>>,
    /// Optional cron schedule store exposed via `/schedules` (+ firing loop).
    pub schedules: Option<Arc<ScheduleStore>>,
    /// Completed background schedule runs waiting for the desktop to surface.
    pub schedule_runs: Arc<ScheduleRunQueue>,
    /// Optional skills store exposed via `/skills`.
    pub skills: Option<Arc<SkillStore>>,
    /// Optional multi-provider registry exposed via `/providers`.
    pub providers: Option<Arc<crate::providers::ProviderRegistry>>,
    /// Selected-file Google Workspace connection used by internal desktop APIs
    /// and the Drive/Docs/Sheets/Slides agent tools.
    pub google_workspace: Option<Arc<crate::google_workspace::GoogleWorkspaceConnection>>,
    /// Durable local media metadata and generated files.
    pub media_library: Option<Arc<MediaLibrary>>,
    /// Host working-folder root shared with the desktop's filesystem/shell
    /// tools. Set via `POST /workspace` (the GUI's "Folder" chip). When unset,
    /// host tools refuse to run. Shared (not cloned) across handlers + tools.
    pub workspace: Arc<RwLock<Option<PathBuf>>>,
    /// One-shot user decisions for consequential streamed tool calls.
    pub tool_approvals: Arc<milim_agents::ToolApprovalBroker>,
    pub(crate) account_runtime_tools: Arc<Mutex<HashMap<String, AccountRuntimeToolSession>>>,
    /// Optional MCP client hub: external MCP servers whose tools are merged
    /// into the agent's registry. Managed via `/mcp/servers`.
    pub mcp: Option<Arc<milim_mcp_client::McpHub>>,
    /// Ephemeral validated HTML served to opaque-origin MCP App iframes.
    pub(crate) mcp_app_views: Arc<McpAppViewStore>,
    /// Computer-use gate (screen capture + mouse/keyboard). Off by default;
    /// flipped via `POST /computer`. The desktop's computer-use tools check it.
    pub computer_use: Arc<AtomicBool>,
    /// Outbound privacy gate. Off by default; set via `POST /privacy/mode`. The
    /// `ProviderRouter` consults it before sending to a remote provider.
    pub privacy: Arc<crate::privacy::PrivacyGate>,
    /// Native-mobile pairing and per-device credential store. Disabled by
    /// default; canonical state and mutations flow through `/control/v1`.
    pub mobile_companion: Option<Arc<MobileCompanionBridge>>,
    /// Canonical server-owned user thread/run/timeline state used by desktop
    /// and native mobile control clients.
    pub control: Option<Arc<RunManager>>,
    /// Set only on the separately bound phone router. Prevents loopback trust
    /// or desktop API keys from widening that router beyond paired devices.
    pub(crate) mobile_control_only: bool,
    /// Managed preview app runtime; no-folder staged apps live under `~/.milim/runtime`.
    pub preview_runtime: Arc<PreviewRuntimeManager>,
}

impl AppState {
    /// Build state with auth disabled and loopback trusted (dev defaults).
    pub fn new(service: SharedService, config: ServerConfiguration) -> Self {
        Self {
            service,
            config: Arc::new(config),
            api_keys: Arc::new(HashSet::new()),
            trust_loopback: true,
            created: crate::now_unix(),
            access_validator: None,
            tools: None,
            memory: None,
            agents: None,
            threads: None,
            schedules: None,
            schedule_runs: Arc::new(ScheduleRunQueue::default()),
            skills: None,
            providers: None,
            google_workspace: None,
            media_library: None,
            workspace: Arc::new(RwLock::new(None)),
            tool_approvals: Arc::new(milim_agents::ToolApprovalBroker::default()),
            account_runtime_tools: Arc::new(Mutex::new(HashMap::new())),
            mcp: None,
            mcp_app_views: Arc::new(McpAppViewStore::default()),
            computer_use: Arc::new(AtomicBool::new(false)),
            privacy: Arc::new(crate::privacy::PrivacyGate::default()),
            mobile_companion: None,
            control: None,
            mobile_control_only: false,
            preview_runtime: Arc::new(PreviewRuntimeManager::new(
                milim_core::paths::Paths::resolve()
                    .root()
                    .join("runtime")
                    .join("preview-apps"),
            )),
        }
    }

    /// Share the computer-use gate with the desktop's computer-use tools.
    pub fn with_computer_use(mut self, gate: Arc<AtomicBool>) -> Self {
        self.computer_use = gate;
        self
    }

    /// Share the outbound privacy gate with the provider router (so the
    /// `/privacy/mode` endpoint and the router read the same mode).
    pub fn with_privacy(mut self, gate: Arc<crate::privacy::PrivacyGate>) -> Self {
        self.privacy = gate;
        self
    }

    /// Attach the native-mobile pairing and credential store.
    pub fn with_mobile_companion(mut self, bridge: Arc<MobileCompanionBridge>) -> Self {
        self.mobile_companion = Some(bridge);
        self
    }

    pub fn with_control(mut self, manager: Arc<RunManager>) -> Self {
        self.control = Some(manager);
        self
    }

    /// Share a working-folder cell with host filesystem/shell tools so the
    /// `/workspace` endpoint and the tools read/write the same root.
    pub fn with_workspace(mut self, workspace: Arc<RwLock<Option<PathBuf>>>) -> Self {
        self.workspace = workspace;
        self
    }

    /// Attach an MCP client hub (external MCP servers' tools).
    pub fn with_mcp(mut self, mcp: Arc<milim_mcp_client::McpHub>) -> Self {
        self.mcp = Some(mcp);
        self
    }

    /// Attach a multi-provider registry for the `/providers` endpoints.
    pub fn with_providers(mut self, providers: Arc<crate::providers::ProviderRegistry>) -> Self {
        self.providers = Some(providers);
        self
    }

    /// Attach the selected-file Google Workspace connector.
    pub fn with_google_workspace(
        mut self,
        google_workspace: Arc<crate::google_workspace::GoogleWorkspaceConnection>,
    ) -> Self {
        self.google_workspace = Some(google_workspace);
        self
    }

    /// Attach the durable local media library.
    pub fn with_media_library(mut self, library: MediaLibrary) -> Self {
        self.media_library = Some(Arc::new(library));
        self
    }

    /// Attach a skills store.
    pub fn with_skills(mut self, skills: SkillStore) -> Self {
        self.skills = Some(Arc::new(skills));
        self
    }

    /// Attach a named-agent store.
    pub fn with_agents(mut self, agents: AgentStore) -> Self {
        self.agents = Some(Arc::new(agents));
        self
    }

    /// Attach the child-thread supervisor.
    pub fn with_threads(mut self, threads: ThreadSupervisor) -> Self {
        self.threads = Some(Arc::new(threads));
        self
    }

    /// Attach a cron schedule store.
    pub fn with_schedules(mut self, schedules: ScheduleStore) -> Self {
        self.schedules = Some(Arc::new(schedules));
        self
    }

    /// Attach a tool registry for the MCP endpoints.
    pub fn with_tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = Some(Arc::new(tools));
        self
    }

    /// Attach a memory/RAG store for the memory endpoints.
    pub fn with_memory(mut self, memory: MemoryStore) -> Self {
        self.memory = Some(Arc::new(memory));
        self
    }

    /// Attach an msk-v1 access-key validator.
    pub fn with_access_validator(mut self, validator: AccessKeyValidator) -> Self {
        self.access_validator = Some(Arc::new(validator));
        self
    }

    /// Replace the accepted API-key set.
    pub fn with_api_keys(mut self, keys: impl IntoIterator<Item = String>) -> Self {
        self.api_keys = Arc::new(keys.into_iter().collect());
        self
    }

    /// Toggle loopback auth bypass (off forces key checks even on localhost).
    pub fn with_loopback_trust(mut self, trust: bool) -> Self {
        self.trust_loopback = trust;
        self
    }
}
