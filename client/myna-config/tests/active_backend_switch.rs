use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use async_trait::async_trait;
use myna_config::active_backend::{
    ensure_backend_active, execute_switch, myna_restart_request, ActiveBackendController,
    PrepareSwitchError, SnapdWait, SwitchNotice, SwitchOutcome, SwitchPlan,
};
use myna_config::backend_apply::ApplyPreview;
use myna_config::command::{CancellationToken, CommandRequest};
use myna_config::domain::{
    parse_connections, ActiveBackendState, BackendIdentity, BackendSnapshot, BackendSurface,
    BackendSurfaceError, CommandResult, ConnectionSnapshot,
};
use myna_config::operation_gate::{OperationCoordinator, OperationKind};
use myna_config::ports::{
    BackendRepository, SystemConfigurator, SystemConfiguratorError, SystemConfiguratorFailure,
};
use myna_config::snap_changes::{parse_in_progress, SnapChange};

fn connections(snaps: &[&str], connected: &[&str]) -> ConnectionSnapshot {
    let mut rows = String::from("Interface Plug Slot Notes\n");
    let mut slots = String::from("name: content\nslots:\n");
    for snap in snaps {
        if connected.contains(snap) {
            rows.push_str(&format!(
                "content[inference-provider] myna:backend {snap}:provider manual\n"
            ));
        } else {
            rows.push_str(&format!("content - {snap}:provider -\n"));
        }
        slots.push_str(&format!(
            "  - {snap}:provider:\n      content: inference-provider\n"
        ));
    }
    parse_connections(&rows, &slots).unwrap()
}

fn argv(plan: &SwitchPlan) -> Vec<(&str, Vec<&str>)> {
    plan.operations()
        .iter()
        .map(|request| {
            (
                request.executable(),
                request.arguments().iter().map(String::as_str).collect(),
            )
        })
        .collect()
}

#[test]
fn plan_connects_from_zero_connections() {
    let plan = SwitchPlan::new(
        &connections(&["myna-parakeet"], &[]),
        BackendIdentity::new("myna-parakeet", "provider"),
    )
    .unwrap();
    assert_eq!(
        argv(&plan),
        [
            (
                "snap",
                vec!["connect", "myna:backend", "myna-parakeet:provider"]
            ),
            (
                "systemctl",
                vec!["--user", "restart", "snap.myna.myna.service"]
            )
        ]
    );
}

#[test]
fn plan_switches_one_connection_disconnect_first() {
    let plan = SwitchPlan::new(
        &connections(&["myna-parakeet", "myna-whisper"], &["myna-parakeet"]),
        BackendIdentity::new("myna-whisper", "provider"),
    )
    .unwrap();
    assert_eq!(
        argv(&plan),
        [
            (
                "snap",
                vec!["disconnect", "myna:backend", "myna-parakeet:provider"]
            ),
            (
                "snap",
                vec!["connect", "myna:backend", "myna-whisper:provider"]
            ),
            (
                "systemctl",
                vec!["--user", "restart", "snap.myna.myna.service"]
            )
        ]
    );
}

#[test]
fn plan_disconnects_every_multiple_connection_before_connecting() {
    let plan = SwitchPlan::new(
        &connections(
            &["myna-parakeet", "myna-whisper", "other"],
            &["myna-parakeet", "myna-whisper"],
        ),
        BackendIdentity::new("other", "provider"),
    )
    .unwrap();
    assert_eq!(plan.operations().len(), 4);
    assert_eq!(plan.operations()[0].arguments()[0], "disconnect");
    assert_eq!(plan.operations()[1].arguments()[0], "disconnect");
    assert_eq!(plan.operations()[2].arguments()[0], "connect");
    assert_eq!(plan.operations()[3], myna_restart_request());
}

#[test]
fn same_backend_as_exactly_one_connection_is_noop() {
    let plan = SwitchPlan::new(
        &connections(&["myna-parakeet"], &["myna-parakeet"]),
        BackendIdentity::new("myna-parakeet", "provider"),
    )
    .unwrap();
    assert!(plan.operations().is_empty());
    assert!(plan.is_noop());
}

#[test]
fn selecting_one_of_multiple_connections_still_converges_to_one() {
    let plan = SwitchPlan::new(
        &connections(
            &["myna-parakeet", "myna-whisper"],
            &["myna-parakeet", "myna-whisper"],
        ),
        BackendIdentity::new("myna-parakeet", "provider"),
    )
    .unwrap();
    assert_eq!(
        argv(&plan),
        [
            (
                "snap",
                vec!["disconnect", "myna:backend", "myna-parakeet:provider"]
            ),
            (
                "snap",
                vec!["disconnect", "myna:backend", "myna-whisper:provider"]
            ),
            (
                "snap",
                vec!["connect", "myna:backend", "myna-parakeet:provider"]
            ),
            (
                "systemctl",
                vec!["--user", "restart", "snap.myna.myna.service"]
            ),
        ]
    );
}

#[test]
fn missing_selected_backend_is_rejected() {
    assert_eq!(
        SwitchPlan::new(
            &connections(&["myna-parakeet"], &[]),
            BackendIdentity::new("vanished", "provider"),
        )
        .unwrap_err(),
        PrepareSwitchError::BackendUnavailable(BackendIdentity::new("vanished", "provider"))
    );
}

type ChangesRead = Result<Vec<SnapChange>, BackendSurfaceError>;

#[derive(Clone)]
struct FakeRepository {
    discoveries: Rc<RefCell<VecDeque<Result<ConnectionSnapshot, BackendSurfaceError>>>>,
    calls: Rc<RefCell<usize>>,
    changes: Rc<RefCell<VecDeque<ChangesRead>>>,
}

impl FakeRepository {
    fn new(
        discoveries: impl IntoIterator<Item = Result<ConnectionSnapshot, BackendSurfaceError>>,
    ) -> Self {
        Self {
            discoveries: Rc::new(RefCell::new(discoveries.into_iter().collect())),
            calls: Rc::new(RefCell::new(0)),
            changes: Rc::default(),
        }
    }

    /// What successive `snap changes` reads report; once exhausted, nothing
    /// is in progress.
    fn with_changes(self, changes: impl IntoIterator<Item = ChangesRead>) -> Self {
        self.changes.borrow_mut().extend(changes);
        self
    }

    fn calls(&self) -> usize {
        *self.calls.borrow()
    }
}

#[async_trait(?Send)]
impl BackendRepository for FakeRepository {
    async fn changes_in_progress(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<Vec<SnapChange>, BackendSurfaceError> {
        self.changes
            .borrow_mut()
            .pop_front()
            .unwrap_or(Ok(Vec::new()))
    }

    async fn discover(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<ConnectionSnapshot, BackendSurfaceError> {
        *self.calls.borrow_mut() += 1;
        self.discoveries.borrow_mut().pop_front().unwrap()
    }

    async fn read_snapshot(
        &self,
        backend: &BackendIdentity,
        _cancellation: CancellationToken,
    ) -> BackendSnapshot {
        BackendSnapshot::empty(backend.clone())
    }

    async fn refresh(
        &self,
        cancellation: CancellationToken,
    ) -> Result<ConnectionSnapshot, BackendSurfaceError> {
        self.discover(cancellation).await
    }
}

#[derive(Clone)]
struct FakeConfigurator {
    result: Rc<RefCell<Option<ConfiguratorResult>>>,
    calls: Rc<RefCell<Vec<Vec<CommandRequest>>>>,
    restarts: Rc<RefCell<usize>>,
}

type ConfiguratorResult = Result<Vec<CommandResult>, SystemConfiguratorFailure>;

impl FakeConfigurator {
    fn returning(result: ConfiguratorResult) -> Self {
        Self {
            result: Rc::new(RefCell::new(Some(result))),
            calls: Rc::new(RefCell::new(Vec::new())),
            restarts: Rc::new(RefCell::new(0)),
        }
    }

    fn calls(&self) -> Vec<Vec<CommandRequest>> {
        self.calls.borrow().clone()
    }
}

#[async_trait(?Send)]
impl SystemConfigurator for FakeConfigurator {
    async fn execute_backend_switch(
        &self,
        plan: &SwitchPlan,
        _cancellation: CancellationToken,
    ) -> Result<Vec<CommandResult>, SystemConfiguratorFailure> {
        self.calls.borrow_mut().push(plan.operations().to_vec());
        self.result.borrow_mut().take().unwrap()
    }

    async fn restart_myna(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<(), SystemConfiguratorError> {
        *self.restarts.borrow_mut() += 1;
        match self.result.borrow_mut().take() {
            Some(Err(failure)) => Err(failure.into_parts().1),
            _ => Ok(()),
        }
    }

    async fn apply_backend_config(
        &self,
        _preview: &ApplyPreview,
        _cancellation: CancellationToken,
    ) -> Result<Vec<CommandResult>, SystemConfiguratorFailure> {
        unreachable!("a switch never applies backend settings")
    }
}

fn success(plan: &SwitchPlan) -> Vec<CommandResult> {
    plan.operations()
        .iter()
        .map(|request| {
            CommandResult::new(
                request.executable(),
                request.arguments().to_vec(),
                Some(0),
                "",
                "",
            )
        })
        .collect()
}

fn error(message: &str) -> BackendSurfaceError {
    BackendSurfaceError::new(BackendSurface::Connections, message, "")
}

fn no_sleep(_: Duration) -> Pin<Box<dyn Future<Output = ()>>> {
    Box::pin(std::future::ready(()))
}

fn no_wait() -> SnapdWait<'static> {
    SnapdWait {
        interval: Duration::from_secs(2),
        timeout: Duration::from_secs(10),
        sleep: &no_sleep,
    }
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    gtk4::glib::MainContext::new().block_on(future)
}

#[test]
fn execution_rediscovers_before_and_after_and_reports_agreement() {
    let initial = connections(&["old", "new"], &["old"]);
    let final_state = connections(&["old", "new"], &["new"]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("new", "provider")).unwrap();
    let repository = FakeRepository::new([Ok(initial), Ok(final_state.clone())]);
    let configurator = FakeConfigurator::returning(Ok(success(&plan)));

    let outcome = block_on(execute_switch(
        &plan,
        &configurator,
        &repository,
        CancellationToken::new(),
    ));

    assert!(matches!(
        outcome,
        SwitchOutcome::Applied { final_snapshot, .. }
            if final_snapshot == final_state
    ));
    assert_eq!(repository.calls(), 2);
    assert_eq!(configurator.calls(), [plan.operations()]);
}

#[test]
fn stale_discovery_and_disappearing_selection_are_blocked_without_privilege() {
    for current in [
        connections(&["old", "new", "external"], &["old", "external"]),
        connections(&["old"], &["old"]),
    ] {
        let original = connections(&["old", "new"], &["old"]);
        let plan = SwitchPlan::new(&original, BackendIdentity::new("new", "provider")).unwrap();
        let repository = FakeRepository::new([Ok(current.clone())]);
        let configurator = FakeConfigurator::returning(Ok(vec![]));
        let outcome = block_on(execute_switch(
            &plan,
            &configurator,
            &repository,
            CancellationToken::new(),
        ));
        assert!(matches!(
            outcome,
            SwitchOutcome::StaleDiscovery { final_snapshot } if final_snapshot == current
        ));
        assert!(configurator.calls().is_empty());
        assert_eq!(repository.calls(), 1);
    }
}

#[test]
fn cached_noop_is_rechecked_and_external_change_is_reported_without_privilege() {
    let cached = connections(&["old", "new"], &["new"]);
    let changed = connections(&["old", "new"], &["old"]);
    let plan = SwitchPlan::new(&cached, BackendIdentity::new("new", "provider")).unwrap();
    assert!(plan.is_noop());
    let repository = FakeRepository::new([Ok(changed.clone())]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));

    let outcome = block_on(execute_switch(
        &plan,
        &configurator,
        &repository,
        CancellationToken::new(),
    ));

    assert!(matches!(
        outcome,
        SwitchOutcome::StaleDiscovery { final_snapshot } if final_snapshot == changed
    ));
    assert_eq!(repository.calls(), 1);
    assert!(configurator.calls().is_empty());
}

#[test]
fn verified_noop_returns_the_final_reread_state_without_authorization() {
    let cached = connections(&["old", "new"], &["new"]);
    let plan = SwitchPlan::new(&cached, BackendIdentity::new("new", "provider")).unwrap();
    let repository = FakeRepository::new([Ok(cached.clone())]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));

    let outcome = block_on(execute_switch(
        &plan,
        &configurator,
        &repository,
        CancellationToken::new(),
    ));

    assert!(matches!(
        outcome,
        SwitchOutcome::Noop { final_snapshot } if final_snapshot == cached
    ));
    assert_eq!(repository.calls(), 1);
    assert!(configurator.calls().is_empty());
}

#[test]
fn every_operation_failure_and_auth_denial_still_rediscover_actual_state() {
    let initial = connections(&["old", "new"], &["old"]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("new", "provider")).unwrap();
    let disconnected = connections(&["old", "new"], &[]);
    for (completed, error, final_state) in [
        (
            vec![],
            SystemConfiguratorError::execution("pkexec", vec![], Some(1), "disconnect", "failed"),
            initial.clone(),
        ),
        (
            success(&plan)[..1].to_vec(),
            SystemConfiguratorError::execution("pkexec", vec![], Some(1), "connect", "failed"),
            disconnected.clone(),
        ),
        (
            vec![],
            SystemConfiguratorError::authorization_denied("pkexec", vec![], Some(126), "denied"),
            initial.clone(),
        ),
    ] {
        let repository = FakeRepository::new([Ok(initial.clone()), Ok(final_state.clone())]);
        let configurator = FakeConfigurator::returning(Err(SystemConfiguratorFailure::new(
            completed.clone(),
            error.clone(),
        )));
        let outcome = block_on(execute_switch(
            &plan,
            &configurator,
            &repository,
            CancellationToken::new(),
        ));
        assert!(matches!(
            outcome,
            SwitchOutcome::Failed {
                completed: actual_completed,
                error: actual_error,
                final_snapshot: Some(actual),
                ..
            } if actual_completed == completed && actual_error == error && actual == final_state
        ));
        assert_eq!(repository.calls(), 2);
    }
}

#[test]
fn cancellation_rediscovers_without_false_rollback() {
    let initial = connections(&["old", "new"], &["old"]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("new", "provider")).unwrap();
    let final_state = connections(&["old", "new"], &[]);
    let repository = FakeRepository::new([Ok(initial.clone()), Ok(final_state.clone())]);
    let configurator = FakeConfigurator::returning(Err(SystemConfiguratorFailure::new(
        success(&plan)[..1].to_vec(),
        SystemConfiguratorError::Cancelled,
    )));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let outcome = block_on(execute_switch(
        &plan,
        &configurator,
        &repository,
        cancellation,
    ));
    assert!(matches!(
        outcome,
        SwitchOutcome::Cancelled {
            final_snapshot: Some(actual),
            ..
        } if actual == final_state
    ));
    assert!(configurator.calls().is_empty());
}

#[test]
fn a_dismissed_authorization_prompt_is_a_cancel_that_rereads_the_connections() {
    let initial = connections(&["old", "new"], &["old"]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("new", "provider")).unwrap();
    let repository = FakeRepository::new([Ok(initial.clone()), Ok(initial.clone())]);
    let configurator = FakeConfigurator::returning(Err(SystemConfiguratorFailure::new(
        Vec::new(),
        SystemConfiguratorError::Cancelled,
    )));
    let outcome = block_on(execute_switch(
        &plan,
        &configurator,
        &repository,
        CancellationToken::new(),
    ));
    assert!(matches!(
        outcome,
        SwitchOutcome::Cancelled {
            final_snapshot: Some(actual),
            discovery_error: None,
            ..
        } if actual == initial
    ));
    assert_eq!(repository.calls(), 2);
}

#[test]
fn partial_reconciliation_and_post_operation_disagreement_are_honest() {
    let initial = connections(&["old", "new", "external"], &["old", "external"]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("new", "provider")).unwrap();
    let disagreement = connections(&["old", "new", "external"], &["external"]);
    let repository = FakeRepository::new([Ok(initial), Ok(disagreement.clone())]);
    let configurator = FakeConfigurator::returning(Ok(success(&plan)));

    let outcome = block_on(execute_switch(
        &plan,
        &configurator,
        &repository,
        CancellationToken::new(),
    ));
    assert!(matches!(
        outcome,
        SwitchOutcome::Disagreed { final_snapshot, .. } if final_snapshot == disagreement
    ));
}

#[test]
fn failed_final_rediscovery_is_exposed() {
    let initial = connections(&["old", "new"], &["old"]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("new", "provider")).unwrap();
    let repository = FakeRepository::new([Ok(initial), Err(error("refresh failed"))]);
    let configurator = FakeConfigurator::returning(Ok(success(&plan)));
    assert!(matches!(
        block_on(execute_switch(
            &plan,
            &configurator,
            &repository,
            CancellationToken::new(),
        )),
        SwitchOutcome::FinalDiscoveryFailed { .. }
    ));
}

#[test]
fn controller_serializes_operations_ignores_stale_tokens_and_applies_final_discovery() {
    let initial = connections(&["old", "new"], &["old"]);
    let controller =
        ActiveBackendController::with_coordinator(initial.clone(), OperationCoordinator::new());
    let first = controller
        .begin(BackendIdentity::new("new", "provider"))
        .unwrap();
    assert_eq!(
        controller
            .begin(BackendIdentity::new("old", "provider"))
            .unwrap_err(),
        PrepareSwitchError::Busy
    );
    assert_eq!(
        controller
            .begin(BackendIdentity::new("new", "provider"))
            .unwrap_err(),
        PrepareSwitchError::Busy
    );
    let actual = connections(&["old", "new"], &["new"]);

    assert!(controller.complete(
        first.operation_token(),
        SwitchOutcome::Applied {
            completed: vec![],
            final_snapshot: initial.clone(),
        }
    ));
    let second = controller
        .begin(BackendIdentity::new("new", "provider"))
        .unwrap();
    assert!(!controller.complete(
        first.operation_token(),
        SwitchOutcome::Applied {
            completed: vec![],
            final_snapshot: initial,
        }
    ));
    assert!(controller.complete(
        second.operation_token(),
        SwitchOutcome::Applied {
            completed: vec![],
            final_snapshot: actual.clone(),
        }
    ));
    assert_eq!(
        controller.snapshot().active_state(),
        ActiveBackendState::Connected(BackendIdentity::new("new", "provider"))
    );
}

#[test]
fn controller_uses_the_shared_apply_switch_gate() {
    let gate = OperationCoordinator::new();
    let controller = ActiveBackendController::with_coordinator(
        connections(&["old", "new"], &["old"]),
        gate.clone(),
    );
    let apply = gate.begin(OperationKind::BackendApply).unwrap();
    assert_eq!(
        controller
            .begin(BackendIdentity::new("new", "provider"))
            .unwrap_err(),
        PrepareSwitchError::Busy
    );
    assert!(gate.complete(apply.token()));

    let switch = controller
        .begin(BackendIdentity::new("new", "provider"))
        .unwrap();
    assert!(gate.begin(OperationKind::BackendApply).is_err());
    assert_eq!(gate.active(), Some(OperationKind::BackendSwitch));
    assert!(gate.begin(OperationKind::BackendApply).is_err());
    assert!(controller.complete(
        switch.operation_token(),
        SwitchOutcome::Cancelled {
            completed: vec![],
            final_snapshot: Some(connections(&["old", "new"], &["old"])),
            discovery_error: None,
        }
    ));
    assert_eq!(gate.active(), None);
}

#[test]
fn selector_only_marks_a_backend_chosen_for_one_actual_connection() {
    let disconnected = ActiveBackendController::with_coordinator(
        connections(&["myna-parakeet", "myna-whisper"], &[]),
        OperationCoordinator::new(),
    );
    assert_eq!(disconnected.chosen(), None);

    let connected = ActiveBackendController::with_coordinator(
        connections(&["myna-parakeet", "myna-whisper"], &["myna-whisper"]),
        OperationCoordinator::new(),
    );
    assert_eq!(
        connected.chosen(),
        Some(BackendIdentity::new("myna-whisper", "provider"))
    );

    let multiple = ActiveBackendController::with_coordinator(
        connections(
            &["myna-parakeet", "myna-whisper"],
            &["myna-parakeet", "myna-whisper"],
        ),
        OperationCoordinator::new(),
    );
    assert_eq!(multiple.chosen(), None);
}

#[test]
fn a_pending_switch_marks_its_target_chosen_until_it_completes() {
    let controller = ActiveBackendController::with_coordinator(
        connections(&["myna-parakeet", "myna-whisper"], &["myna-parakeet"]),
        OperationCoordinator::new(),
    );
    let whisper = BackendIdentity::new("myna-whisper", "provider");
    assert_eq!(controller.switching_to(), None);
    let switch = controller.begin(whisper.clone()).unwrap();
    assert_eq!(controller.chosen(), Some(whisper.clone()));
    assert_eq!(controller.switching_to(), Some(whisper));

    assert!(controller.complete(
        switch.operation_token(),
        SwitchOutcome::Cancelled {
            completed: vec![],
            final_snapshot: Some(connections(
                &["myna-parakeet", "myna-whisper"],
                &["myna-parakeet"],
            )),
            discovery_error: None,
        }
    ));
    assert_eq!(
        controller.chosen(),
        Some(BackendIdentity::new("myna-parakeet", "provider"))
    );
    assert_eq!(controller.switching_to(), None);

    controller
        .begin(BackendIdentity::new("myna-whisper", "provider"))
        .unwrap();
    controller.abandon();
    assert_eq!(
        controller.chosen(),
        Some(BackendIdentity::new("myna-parakeet", "provider"))
    );
    assert_eq!(controller.switching_to(), None);
}

#[test]
fn only_a_switch_that_did_not_take_is_announced() {
    let snapshot = connections(&["myna-parakeet"], &["myna-parakeet"]);
    let notice = |outcome: SwitchOutcome| outcome.notice();
    let unread = || BackendSurfaceError::new(BackendSurface::Connections, "unreadable", "");
    assert_eq!(
        notice(SwitchOutcome::Applied {
            completed: Vec::new(),
            final_snapshot: snapshot.clone(),
        }),
        SwitchNotice::None
    );
    assert_eq!(
        notice(SwitchOutcome::Noop {
            final_snapshot: snapshot.clone(),
        }),
        SwitchNotice::None
    );
    assert_eq!(
        notice(SwitchOutcome::Cancelled {
            completed: Vec::new(),
            final_snapshot: None,
            discovery_error: Some(unread()),
        }),
        SwitchNotice::None
    );
    assert_eq!(
        notice(SwitchOutcome::Failed {
            completed: Vec::new(),
            error: SystemConfiguratorError::authorization_denied("snap", Vec::new(), None, ""),
            final_snapshot: Some(snapshot.clone()),
            discovery_error: None,
        }),
        SwitchNotice::Failed
    );
    assert_eq!(
        notice(SwitchOutcome::Disagreed {
            completed: Vec::new(),
            final_snapshot: snapshot.clone(),
        }),
        SwitchNotice::Failed
    );
    assert_eq!(
        notice(SwitchOutcome::StaleDiscovery {
            final_snapshot: snapshot,
        }),
        SwitchNotice::Failed
    );
    assert_eq!(
        notice(SwitchOutcome::FinalDiscoveryFailed {
            completed: Vec::new(),
            error: unread(),
        }),
        SwitchNotice::Unconfirmed
    );
}

/// A store install auto-connects a same-publisher backend, and the daemon may
/// have started before it existed: only the restart is left to do.
#[test]
fn an_auto_connected_backend_is_only_restarted() {
    let repository = FakeRepository::new([Ok(connections(&["myna-parakeet"], &["myna-parakeet"]))]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));

    block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap();

    assert!(configurator.calls().is_empty());
    assert_eq!(*configurator.restarts.borrow(), 1);
}

#[test]
fn a_failed_restart_is_reported() {
    let repository = FakeRepository::new([Ok(connections(&["myna-parakeet"], &["myna-parakeet"]))]);
    let configurator = FakeConfigurator::returning(Err(SystemConfiguratorFailure::new(
        Vec::new(),
        SystemConfiguratorError::execution("systemctl", vec![], Some(5), "", "unit not found"),
    )));

    let error = block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap_err();

    assert!(error.contains("unit not found"), "{error}");
}

#[test]
fn an_unconnected_machine_switches_to_the_preferred_backend() {
    let initial = connections(&["myna-whisper", "myna-parakeet"], &[]);
    let connected = connections(&["myna-whisper", "myna-parakeet"], &["myna-parakeet"]);
    let repository = FakeRepository::new([Ok(initial.clone()), Ok(initial.clone()), Ok(connected)]);
    let plan =
        SwitchPlan::new(&initial, BackendIdentity::new("myna-parakeet", "provider")).unwrap();
    let configurator = FakeConfigurator::returning(Ok(success(&plan)));

    block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap();

    assert_eq!(configurator.calls(), [plan.operations()]);
    assert_eq!(*configurator.restarts.borrow(), 0);
}

#[test]
fn without_the_preferred_backend_the_first_discovered_one_is_used() {
    let initial = connections(&["myna-whisper"], &[]);
    let connected = connections(&["myna-whisper"], &["myna-whisper"]);
    let repository = FakeRepository::new([Ok(initial.clone()), Ok(initial.clone()), Ok(connected)]);
    let plan = SwitchPlan::new(&initial, BackendIdentity::new("myna-whisper", "provider")).unwrap();
    let configurator = FakeConfigurator::returning(Ok(success(&plan)));

    block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap();

    assert_eq!(configurator.calls(), [plan.operations()]);
}

#[test]
fn no_backend_and_failed_discovery_are_errors_without_privilege() {
    for discovery in [Ok(connections(&[], &[])), Err(error("snapd is down"))] {
        let repository = FakeRepository::new([discovery]);
        let configurator = FakeConfigurator::returning(Ok(vec![]));

        assert!(block_on(ensure_backend_active(
            &repository,
            &configurator,
            "myna-parakeet",
            &no_wait()
        ))
        .is_err());
        assert!(configurator.calls().is_empty());
        assert_eq!(*configurator.restarts.borrow(), 0);
    }
}

#[test]
fn a_failed_or_contradicted_switch_is_reported() {
    let initial = connections(&["myna-parakeet"], &[]);
    let plan =
        SwitchPlan::new(&initial, BackendIdentity::new("myna-parakeet", "provider")).unwrap();
    let denied = FakeConfigurator::returning(Err(SystemConfiguratorFailure::new(
        Vec::new(),
        SystemConfiguratorError::authorization_denied("snapd", vec![], Some(403), "cancelled"),
    )));
    let repository = FakeRepository::new([
        Ok(initial.clone()),
        Ok(initial.clone()),
        Ok(initial.clone()),
    ]);
    assert!(block_on(ensure_backend_active(
        &repository,
        &denied,
        "myna-parakeet",
        &no_wait()
    ))
    .is_err());

    // snapd reported success but the backend is still not connected.
    let contradicted = FakeConfigurator::returning(Ok(success(&plan)));
    let repository = FakeRepository::new([Ok(initial.clone()), Ok(initial.clone()), Ok(initial)]);
    let error = block_on(ensure_backend_active(
        &repository,
        &contradicted,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap_err();
    assert!(
        error.contains("changed while it was being enabled"),
        "{error}"
    );
}

#[test]
fn a_switch_lost_to_discovery_or_cancellation_is_reported() {
    let initial = connections(&["myna-parakeet"], &[]);

    let repository = FakeRepository::new([Ok(initial.clone()), Err(error("snapd went away"))]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));
    let lost = block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap_err();
    assert!(lost.contains("snapd went away"), "{lost}");

    let repository = FakeRepository::new([Ok(initial.clone()), Ok(initial.clone()), Ok(initial)]);
    let configurator = FakeConfigurator::returning(Err(SystemConfiguratorFailure::new(
        Vec::new(),
        SystemConfiguratorError::Cancelled,
    )));
    let cancelled = block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap_err();
    assert!(cancelled.contains("cancelled"), "{cancelled}");
}

/// `snap changes --abs-time` on the machine the race was seen on, seconds
/// after the model's install change auto-connected `myna:backend`.
const INSTALLING: &str = include_str!("fixtures/snap-changes-installing.txt");

/// The wizard saw `myna:backend` connected while snapd was still installing
/// the model, restarted the daemon at once, and the daemon never found the
/// backend mount snapd applied seconds later.
#[test]
fn an_auto_connection_is_restarted_only_once_its_change_is_done() {
    let repository = FakeRepository::new([
        Ok(connections(&["myna-parakeet"], &["myna-parakeet"])),
        Ok(connections(&["myna-parakeet"], &["myna-parakeet"])),
    ])
    .with_changes([
        Ok(parse_in_progress(INSTALLING)),
        Ok(parse_in_progress(INSTALLING)),
    ]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));
    let sleeps = RefCell::new(Vec::new());
    let sleep = |interval: Duration| -> Pin<Box<dyn Future<Output = ()>>> {
        sleeps
            .borrow_mut()
            .push((interval, *configurator.restarts.borrow()));
        Box::pin(std::future::ready(()))
    };
    let wait = SnapdWait {
        interval: Duration::from_secs(2),
        timeout: Duration::from_secs(10),
        sleep: &sleep,
    };

    block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &wait,
    ))
    .unwrap();

    assert_eq!(
        *sleeps.borrow(),
        [(Duration::from_secs(2), 0), (Duration::from_secs(2), 0)]
    );
    assert_eq!(*configurator.restarts.borrow(), 1);
    assert!(repository.changes.borrow().is_empty());
}

/// Before its install change auto-connects the backend, the wizard would
/// otherwise connect it itself and cost a polkit prompt.
#[test]
fn a_connection_its_change_makes_meanwhile_is_not_made_again() {
    let repository = FakeRepository::new([
        Ok(connections(&["myna-parakeet"], &[])),
        Ok(connections(&["myna-parakeet"], &["myna-parakeet"])),
    ])
    .with_changes([Ok(parse_in_progress(INSTALLING))]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));

    block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap();

    assert!(configurator.calls().is_empty());
    assert_eq!(*configurator.restarts.borrow(), 1);
}

#[test]
fn changes_to_other_snaps_are_not_waited_for() {
    let repository = FakeRepository::new([Ok(connections(&["myna-parakeet"], &["myna-parakeet"]))])
        .with_changes([Ok(parse_in_progress(
            "7 Doing 2026-09-28T09:37:28+01:00 - Auto-refresh snap \"firefox\"",
        ))]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));
    let sleep = |_: Duration| -> Pin<Box<dyn Future<Output = ()>>> {
        panic!("waited for a change to another snap")
    };
    let wait = SnapdWait {
        sleep: &sleep,
        ..no_wait()
    };

    block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &wait,
    ))
    .unwrap();

    assert_eq!(*configurator.restarts.borrow(), 1);
}

#[test]
fn a_change_that_outlasts_the_wait_is_reported_without_a_restart() {
    let repository = FakeRepository::new([Ok(connections(&["myna-parakeet"], &["myna-parakeet"]))])
        .with_changes((0..10).map(|_| Ok(parse_in_progress(INSTALLING))));
    let configurator = FakeConfigurator::returning(Ok(vec![]));

    let error = block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap_err();

    assert!(error.contains("Install \"myna-parakeet\" snap"), "{error}");
    assert_eq!(*configurator.restarts.borrow(), 0);
    // 10 s at 2 s: the first read, then one after each of five sleeps.
    assert_eq!(repository.changes.borrow().len(), 4);
}

#[test]
fn unreadable_changes_are_reported_without_a_restart() {
    let repository = FakeRepository::new([Ok(connections(&["myna-parakeet"], &["myna-parakeet"]))])
        .with_changes([Err(error("snapd is down"))]);
    let configurator = FakeConfigurator::returning(Ok(vec![]));

    let failed = block_on(ensure_backend_active(
        &repository,
        &configurator,
        "myna-parakeet",
        &no_wait(),
    ))
    .unwrap_err();

    assert!(failed.contains("snapd is down"), "{failed}");
    assert_eq!(*configurator.restarts.borrow(), 0);
}
