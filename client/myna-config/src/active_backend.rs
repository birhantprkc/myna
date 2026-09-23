use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::command::{CancellationToken, CommandRequest};
use crate::domain::{
    ActiveBackendState, BackendIdentity, BackendSurfaceError, CommandResult, ConnectionSnapshot,
};
use crate::operation_gate::{OperationCoordinator, OperationKind};
use crate::ports::{
    BackendRepository, SystemConfigurator, SystemConfiguratorError, SystemConfiguratorFailure,
};

/// Myna's user service. Restarting it through the user's own systemd needs
/// no authorization, where `snap restart` costs a second polkit prompt.
pub const MYNA_USER_UNIT: &str = "snap.myna.myna.service";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrepareSwitchError {
    BackendUnavailable(BackendIdentity),
    Busy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendHealth {
    Healthy,
    Degraded,
    #[default]
    Unknown,
}

impl BackendHealth {
    pub fn label(self) -> String {
        match self {
            Self::Healthy => gettextrs::gettext("Healthy"),
            Self::Degraded => gettextrs::gettext("Degraded"),
            Self::Unknown => gettextrs::gettext("Health unknown"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendOption {
    backend: BackendIdentity,
    installed: bool,
    connected: bool,
    health: BackendHealth,
}

impl BackendOption {
    pub fn backend(&self) -> &BackendIdentity {
        &self.backend
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn health(&self) -> BackendHealth {
        self.health
    }

    pub fn context(&self) -> String {
        format!(
            "{} • {} • {}",
            gettextrs::gettext("Installed"),
            if self.connected {
                gettextrs::gettext("Connected")
            } else {
                gettextrs::gettext("Not connected")
            },
            self.health.label()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchPlan {
    baseline: ConnectionSnapshot,
    selected: BackendIdentity,
    operations: Vec<CommandRequest>,
    confirmation_text: String,
}

impl SwitchPlan {
    pub fn new(
        snapshot: &ConnectionSnapshot,
        selected: BackendIdentity,
    ) -> Result<Self, PrepareSwitchError> {
        if !snapshot.backends().contains(&selected) {
            return Err(PrepareSwitchError::BackendUnavailable(selected));
        }

        let connected = connected_backends(snapshot);
        let operations = if connected.as_slice() == [selected.clone()] {
            Vec::new()
        } else {
            let mut operations = connected
                .iter()
                .map(|backend| privileged_snap_request("disconnect", backend.slot()))
                .collect::<Vec<_>>();
            operations.push(privileged_snap_request("connect", selected.slot()));
            operations.push(myna_restart_request());
            operations
        };
        let confirmation_text = confirmation_text(&selected);
        Ok(Self {
            baseline: snapshot.clone(),
            selected,
            operations,
            confirmation_text,
        })
    }

    pub fn baseline(&self) -> &ConnectionSnapshot {
        &self.baseline
    }

    pub fn selected(&self) -> &BackendIdentity {
        &self.selected
    }

    pub fn operations(&self) -> &[CommandRequest] {
        &self.operations
    }

    pub fn confirmation_text(&self) -> &str {
        &self.confirmation_text
    }

    pub fn is_noop(&self) -> bool {
        self.operations.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn with_operations_for_test(
        baseline: ConnectionSnapshot,
        selected: BackendIdentity,
        operations: Vec<CommandRequest>,
    ) -> Self {
        let confirmation_text = confirmation_text(&selected);
        Self {
            baseline,
            selected,
            operations,
            confirmation_text,
        }
    }
}

fn privileged_snap_request(action: &str, slot: String) -> CommandRequest {
    CommandRequest::new(
        "snap".to_owned(),
        vec![action.to_owned(), "myna:backend".to_owned(), slot],
    )
}

pub fn myna_restart_request() -> CommandRequest {
    CommandRequest::new(
        "systemctl".to_owned(),
        vec![
            "--user".to_owned(),
            "restart".to_owned(),
            MYNA_USER_UNIT.to_owned(),
        ],
    )
}

fn confirmation_text(selected: &BackendIdentity) -> String {
    gettextrs::gettext("Dictation will use {backend}. It pauses briefly while Myna restarts.")
        .replace("{backend}", selected.snap_name())
}

fn connected_backends(snapshot: &ConnectionSnapshot) -> Vec<BackendIdentity> {
    match snapshot.active_state() {
        ActiveBackendState::Disconnected => Vec::new(),
        ActiveBackendState::Connected(backend) => vec![backend],
        ActiveBackendState::MultiplyConnected(backends) => backends,
        ActiveBackendState::FailedSwitch { observed, .. } => match observed {
            crate::domain::ConnectionState::Disconnected => Vec::new(),
            crate::domain::ConnectionState::Connected(backend) => vec![backend],
            crate::domain::ConnectionState::MultiplyConnected(backends) => backends,
        },
    }
}

fn selected_is_active(snapshot: &ConnectionSnapshot, selected: &BackendIdentity) -> bool {
    matches!(
        snapshot.active_state(),
        ActiveBackendState::Connected(ref backend) if backend == selected
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchOutcome {
    Applied {
        completed: Vec<CommandResult>,
        final_snapshot: ConnectionSnapshot,
    },
    Disagreed {
        completed: Vec<CommandResult>,
        final_snapshot: ConnectionSnapshot,
    },
    StaleDiscovery {
        final_snapshot: ConnectionSnapshot,
    },
    Noop {
        final_snapshot: ConnectionSnapshot,
    },
    Failed {
        completed: Vec<CommandResult>,
        error: SystemConfiguratorError,
        final_snapshot: Option<ConnectionSnapshot>,
        discovery_error: Option<BackendSurfaceError>,
    },
    Cancelled {
        completed: Vec<CommandResult>,
        final_snapshot: Option<ConnectionSnapshot>,
        discovery_error: Option<BackendSurfaceError>,
    },
    FinalDiscoveryFailed {
        completed: Vec<CommandResult>,
        error: BackendSurfaceError,
    },
}

impl SwitchOutcome {
    pub fn final_snapshot(&self) -> Option<&ConnectionSnapshot> {
        match self {
            Self::Applied { final_snapshot, .. }
            | Self::Disagreed { final_snapshot, .. }
            | Self::StaleDiscovery { final_snapshot }
            | Self::Noop { final_snapshot } => Some(final_snapshot),
            Self::Failed { final_snapshot, .. } | Self::Cancelled { final_snapshot, .. } => {
                final_snapshot.as_ref()
            }
            Self::FinalDiscoveryFailed { .. } => None,
        }
    }
}

pub async fn execute_switch(
    plan: &SwitchPlan,
    confirmed: bool,
    configurator: &dyn SystemConfigurator,
    repository: &dyn BackendRepository,
    cancellation: CancellationToken,
) -> SwitchOutcome {
    if !confirmed {
        return cancelled_after_refresh(Vec::new(), repository).await;
    }

    let preflight = match repository.refresh(CancellationToken::new()).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return SwitchOutcome::FinalDiscoveryFailed {
                completed: Vec::new(),
                error,
            }
        }
    };

    if plan.is_noop() {
        if &preflight == plan.baseline() && selected_is_active(&preflight, plan.selected()) {
            return SwitchOutcome::Noop {
                final_snapshot: preflight,
            };
        }
        return SwitchOutcome::StaleDiscovery {
            final_snapshot: preflight,
        };
    }

    if &preflight != plan.baseline() || !preflight.backends().contains(plan.selected()) {
        return SwitchOutcome::StaleDiscovery {
            final_snapshot: preflight,
        };
    }
    if cancellation.is_cancelled() {
        return cancelled_after_refresh(Vec::new(), repository).await;
    }

    let execution = configurator
        .execute_backend_switch(plan, cancellation.clone())
        .await;
    match execution {
        Ok(completed) => match repository.refresh(CancellationToken::new()).await {
            Ok(final_snapshot) if selected_is_active(&final_snapshot, plan.selected()) => {
                SwitchOutcome::Applied {
                    completed,
                    final_snapshot,
                }
            }
            Ok(final_snapshot) => SwitchOutcome::Disagreed {
                completed,
                final_snapshot,
            },
            Err(error) => SwitchOutcome::FinalDiscoveryFailed { completed, error },
        },
        Err(failure) => failed_after_refresh(failure, repository).await,
    }
}

/// Leave dictation running on a backend. With none connected, switch to
/// `preferred`, or the first discovered backend without it; the switch
/// restarts Myna. With one connected, only restart, because the daemon may
/// have started before the backend was installed.
pub async fn ensure_backend_active(
    repository: &dyn BackendRepository,
    configurator: &dyn SystemConfigurator,
    preferred: &str,
) -> Result<(), String> {
    let snapshot = repository
        .refresh(CancellationToken::new())
        .await
        .map_err(|error| error.message().to_owned())?;
    if let ActiveBackendState::Connected(_) = snapshot.active_state() {
        return configurator
            .restart_myna(CancellationToken::new())
            .await
            .map_err(|error| error.to_string());
    }
    let backends = snapshot.backends();
    let Some(selected) = backends
        .iter()
        .find(|backend| backend.snap_name() == preferred)
        .or_else(|| backends.first())
        .cloned()
    else {
        return Err(gettextrs::gettext(
            "No speech-to-text model appeared in snap connections.",
        ));
    };
    let plan = SwitchPlan::new(&snapshot, selected)
        .map_err(|_| gettextrs::gettext("The model is not available to switch to."))?;
    match execute_switch(
        &plan,
        true,
        configurator,
        repository,
        CancellationToken::new(),
    )
    .await
    {
        SwitchOutcome::Applied { .. } | SwitchOutcome::Noop { .. } => Ok(()),
        SwitchOutcome::Failed { error, .. } => Err(error.to_string()),
        SwitchOutcome::FinalDiscoveryFailed { error, .. } => Err(error.message().to_owned()),
        SwitchOutcome::Cancelled { .. } => Err(gettextrs::gettext("The change was cancelled.")),
        SwitchOutcome::Disagreed { .. } | SwitchOutcome::StaleDiscovery { .. } => Err(
            gettextrs::gettext("The backend changed while it was being enabled."),
        ),
    }
}

async fn cancelled_after_refresh(
    completed: Vec<CommandResult>,
    repository: &dyn BackendRepository,
) -> SwitchOutcome {
    match repository.refresh(CancellationToken::new()).await {
        Ok(snapshot) => SwitchOutcome::Cancelled {
            completed,
            final_snapshot: Some(snapshot),
            discovery_error: None,
        },
        Err(error) => SwitchOutcome::Cancelled {
            completed,
            final_snapshot: None,
            discovery_error: Some(error),
        },
    }
}

async fn failed_after_refresh(
    failure: SystemConfiguratorFailure,
    repository: &dyn BackendRepository,
) -> SwitchOutcome {
    let (completed, error) = failure.into_parts();
    let final_discovery = repository.refresh(CancellationToken::new()).await;
    if error == SystemConfiguratorError::Cancelled {
        return match final_discovery {
            Ok(snapshot) => SwitchOutcome::Cancelled {
                completed,
                final_snapshot: Some(snapshot),
                discovery_error: None,
            },
            Err(error) => SwitchOutcome::Cancelled {
                completed,
                final_snapshot: None,
                discovery_error: Some(error),
            },
        };
    }
    match final_discovery {
        Ok(snapshot) => SwitchOutcome::Failed {
            completed,
            error,
            final_snapshot: Some(snapshot),
            discovery_error: None,
        },
        Err(discovery_error) => SwitchOutcome::Failed {
            completed,
            error,
            final_snapshot: None,
            discovery_error: Some(discovery_error),
        },
    }
}

#[derive(Clone, Debug)]
pub struct SwitchRequest {
    operation_token: u64,
    plan: SwitchPlan,
    cancellation: CancellationToken,
}

impl SwitchRequest {
    pub fn operation_token(&self) -> u64 {
        self.operation_token
    }

    pub fn plan(&self) -> &SwitchPlan {
        &self.plan
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

pub struct ActiveBackendController {
    inner: RefCell<ControllerState>,
    coordinator: OperationCoordinator,
}

struct ControllerState {
    snapshot: ConnectionSnapshot,
    verified: bool,
    health: BTreeMap<String, BackendHealth>,
    generation: u64,
    pending: Option<(u64, CancellationToken)>,
    last_outcome: Option<SwitchOutcome>,
}

impl ActiveBackendController {
    pub fn new(snapshot: ConnectionSnapshot) -> Self {
        Self::with_coordinator(snapshot, OperationCoordinator::new())
    }

    pub fn with_coordinator(
        snapshot: ConnectionSnapshot,
        coordinator: OperationCoordinator,
    ) -> Self {
        Self {
            inner: RefCell::new(ControllerState {
                snapshot,
                verified: true,
                health: BTreeMap::new(),
                generation: 0,
                pending: None,
                last_outcome: None,
            }),
            coordinator,
        }
    }

    pub fn snapshot(&self) -> ConnectionSnapshot {
        self.inner.borrow().snapshot.clone()
    }

    pub fn set_snapshot(&self, snapshot: ConnectionSnapshot) -> bool {
        let mut inner = self.inner.borrow_mut();
        if inner.pending.is_some() {
            return false;
        }
        inner.snapshot = snapshot;
        inner.verified = true;
        true
    }

    pub fn set_health(&self, snap: &str, health: BackendHealth) {
        self.inner
            .borrow_mut()
            .health
            .insert(snap.to_owned(), health);
    }

    pub fn options(&self) -> Vec<BackendOption> {
        let inner = self.inner.borrow();
        let connected = connected_backends(&inner.snapshot);
        inner
            .snapshot
            .backends()
            .iter()
            .map(|backend| BackendOption {
                backend: backend.clone(),
                installed: true,
                connected: connected.contains(backend),
                health: inner
                    .health
                    .get(backend.snap_name())
                    .copied()
                    .unwrap_or_default(),
            })
            .collect()
    }

    pub fn selected_index(&self) -> Option<u32> {
        let inner = self.inner.borrow();
        let ActiveBackendState::Connected(active) = inner.snapshot.active_state() else {
            return None;
        };
        inner
            .snapshot
            .backends()
            .iter()
            .position(|backend| backend == &active)
            .map(|index| index as u32)
    }

    pub fn begin(&self, selected: BackendIdentity) -> Result<SwitchRequest, PrepareSwitchError> {
        let mut inner = self.inner.borrow_mut();
        if inner.pending.is_some() {
            return Err(PrepareSwitchError::Busy);
        }
        let plan = SwitchPlan::new(&inner.snapshot, selected)?;
        let operation = self
            .coordinator
            .begin(OperationKind::BackendSwitch)
            .map_err(|_| PrepareSwitchError::Busy)?;
        let operation_token = operation.token();
        let cancellation = operation.cancellation();
        inner.generation = operation_token;
        inner.pending = Some((operation_token, cancellation.clone()));
        Ok(SwitchRequest {
            operation_token,
            plan,
            cancellation,
        })
    }

    pub fn cancel(&self) {
        let inner = self.inner.borrow();
        if let Some((token, _)) = inner.pending.as_ref() {
            self.coordinator.cancel(*token);
        }
    }

    pub fn abandon(&self) {
        let mut inner = self.inner.borrow_mut();
        if let Some((token, _)) = inner.pending.take() {
            self.coordinator.abandon(token);
        }
    }

    pub fn complete(&self, operation_token: u64, outcome: SwitchOutcome) -> bool {
        let mut inner = self.inner.borrow_mut();
        if !matches!(inner.pending, Some((token, _)) if token == operation_token) {
            return false;
        }
        inner.pending = None;
        self.coordinator.complete(operation_token);
        if let Some(snapshot) = outcome.final_snapshot() {
            inner.snapshot = snapshot.clone();
            inner.verified = true;
        } else {
            inner.verified = false;
        }
        inner.last_outcome = Some(outcome);
        true
    }

    pub fn busy(&self) -> bool {
        self.inner.borrow().pending.is_some()
    }

    pub fn verified(&self) -> bool {
        self.inner.borrow().verified
    }

    pub fn last_outcome(&self) -> Option<SwitchOutcome> {
        self.inner.borrow().last_outcome.clone()
    }
}
