use thiserror::Error;

use async_trait::async_trait;

use crate::active_backend::SwitchPlan;
use crate::backend_apply::ApplyPreview;
use crate::command::CancellationToken;
use crate::diagnostics::InstalledSnap;
use crate::domain::{
    BackendIdentity, BackendSnapshot, BackendSurfaceError, ClientSetting, ClientSettingMetadata,
    ClientSettingValue, CommandResult, ConnectionSnapshot,
};
use crate::onboarding::ExtensionState;
use crate::snap_changes::{apply_progress, ApplyProgress, ChangeInProgress};

pub type ClientSettingsCallback = Box<dyn Fn(ClientSetting) + 'static>;

pub trait ClientSettingsSubscription {}

pub trait ClientSettings {
    fn list(&self) -> Result<Vec<ClientSettingMetadata>, ClientSettingsError>;
    fn get(&self, key: &str) -> Result<ClientSettingValue, ClientSettingsError>;
    fn set(&self, key: &str, value: ClientSettingValue) -> Result<(), ClientSettingsError>;
    /// Whether the store holds a value for `key`, rather than the schema
    /// default standing in for one.
    fn has_user_value(&self, key: &str) -> Result<bool, ClientSettingsError>;
    fn subscribe(
        &self,
        callback: ClientSettingsCallback,
    ) -> Result<Box<dyn ClientSettingsSubscription>, ClientSettingsError>;
}

#[async_trait(?Send)]
pub trait BackendRepository {
    async fn installed_snaps(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<Vec<InstalledSnap>, BackendSurfaceError> {
        Ok(Vec::new())
    }

    async fn discover(
        &self,
        cancellation: CancellationToken,
    ) -> Result<ConnectionSnapshot, BackendSurfaceError>;

    async fn read_snapshot(
        &self,
        backend: &BackendIdentity,
        cancellation: CancellationToken,
    ) -> BackendSnapshot;

    async fn refresh(
        &self,
        cancellation: CancellationToken,
    ) -> Result<ConnectionSnapshot, BackendSurfaceError>;
}

#[async_trait(?Send)]
pub trait SystemConfigurator {
    async fn execute_backend_switch(
        &self,
        plan: &SwitchPlan,
        cancellation: CancellationToken,
    ) -> Result<Vec<CommandResult>, SystemConfiguratorFailure>;

    /// Restart Myna's user service so it picks up a changed backend mount.
    async fn restart_myna(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(), SystemConfiguratorError>;

    async fn apply_backend_config(
        &self,
        preview: &ApplyPreview,
        cancellation: CancellationToken,
    ) -> Result<Vec<CommandResult>, SystemConfiguratorFailure>;

    /// The snapd changes that have not finished yet, read as the user.
    async fn changes_in_progress(
        &self,
        _cancellation: CancellationToken,
    ) -> Result<Vec<ChangeInProgress>, String> {
        Ok(Vec::new())
    }

    /// Whether snapd's `experimental.user-daemons` flag is on, read as the
    /// user.
    async fn user_daemons_enabled(&self, cancellation: CancellationToken) -> Result<bool, String>;

    /// Turn snapd's `experimental.user-daemons` flag on, as the user: snapd
    /// raises polkit's prompt itself.
    async fn enable_user_daemons(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(), SystemConfiguratorError>;

    /// What snapd is doing on `backend_snap` now, while an apply runs; none
    /// when it is doing nothing there or cannot be read.
    async fn apply_progress(
        &self,
        backend_snap: &str,
        cancellation: CancellationToken,
    ) -> Option<ApplyProgress> {
        let changes = self.changes_in_progress(cancellation).await.ok()?;
        apply_progress(&changes, backend_snap)
    }
}

/// GNOME Shell's extensions, as the running shell reports them.
#[async_trait(?Send)]
pub trait ShellExtensions {
    /// Where `uuid` stands. A session with no gnome-shell has none.
    async fn extension_state(&self, uuid: &str) -> ExtensionState;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemConfiguratorFailure {
    completed: Vec<CommandResult>,
    error: SystemConfiguratorError,
}

impl SystemConfiguratorFailure {
    pub fn new(completed: Vec<CommandResult>, error: SystemConfiguratorError) -> Self {
        Self { completed, error }
    }

    pub fn completed(&self) -> &[CommandResult] {
        &self.completed
    }

    pub fn error(&self) -> &SystemConfiguratorError {
        &self.error
    }

    pub fn into_parts(self) -> (Vec<CommandResult>, SystemConfiguratorError) {
        (self.completed, self.error)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ClientSettingsError {
    #[error("GSettings schema {schema_id} is unavailable. {guidance}")]
    SchemaUnavailable {
        schema_id: &'static str,
        guidance: &'static str,
    },
    #[error("settings key is not declared by the schema: {key}")]
    UnknownKey { key: String },
    #[error("settings key is not writable: {key}")]
    NotWritable { key: String },
    #[error("invalid value for {key}: {message}")]
    InvalidValue { key: String, message: String },
    #[error("cannot open the Myna settings store: {message}")]
    StoreUnavailable { message: String },
}

/// The privileged step a failure report names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailedStep {
    /// A process run directly or under `pkexec`.
    Command {
        executable: String,
        arguments: Vec<String>,
        exit_status: Option<i32>,
        stderr: String,
    },
    /// A request to snapd's REST API made as the user, such as
    /// `PUT /v2/snaps/system/conf (experimental.user-daemons=true)`; no
    /// status when snapd never answered.
    Snapd {
        request: String,
        http_status: Option<u16>,
    },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SystemConfiguratorError {
    #[error("privileged configuration was cancelled")]
    Cancelled,
    #[error("authorization denied: {message}")]
    AuthorizationDenied { step: FailedStep, message: String },
    #[error("the model rejected the requested values: {message}")]
    ValuesRejected { step: FailedStep, message: String },
    #[error("privileged configuration failed: {message}")]
    Execution { step: FailedStep, message: String },
}

impl SystemConfiguratorError {
    pub fn authorization_denied(
        executable: impl Into<String>,
        arguments: Vec<String>,
        exit_status: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        let (step, message) = command_failure(
            executable,
            arguments,
            exit_status,
            stderr,
            "authorization denied",
        );
        Self::AuthorizationDenied { step, message }
    }

    pub fn values_rejected(
        executable: impl Into<String>,
        arguments: Vec<String>,
        exit_status: Option<i32>,
        stderr: impl Into<String>,
    ) -> Self {
        let (step, message) = command_failure(
            executable,
            arguments,
            exit_status,
            stderr,
            "the model rejected the requested values",
        );
        Self::ValuesRejected { step, message }
    }

    pub fn execution(
        executable: impl Into<String>,
        arguments: Vec<String>,
        exit_status: Option<i32>,
        stderr: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Execution {
            step: FailedStep::Command {
                executable: executable.into(),
                arguments,
                exit_status,
                stderr: stderr.into(),
            },
            message: message.into(),
        }
    }

    pub fn snapd_authorization_denied(
        request: impl Into<String>,
        http_status: u16,
        message: impl Into<String>,
    ) -> Self {
        Self::AuthorizationDenied {
            step: FailedStep::Snapd {
                request: request.into(),
                http_status: Some(http_status),
            },
            message: message.into(),
        }
    }

    pub fn snapd_execution(
        request: impl Into<String>,
        http_status: Option<u16>,
        message: impl Into<String>,
    ) -> Self {
        Self::Execution {
            step: FailedStep::Snapd {
                request: request.into(),
                http_status,
            },
            message: message.into(),
        }
    }

    /// The step that failed; none for a cancellation.
    pub fn step(&self) -> Option<&FailedStep> {
        match self {
            Self::Cancelled => None,
            Self::AuthorizationDenied { step, .. }
            | Self::ValuesRejected { step, .. }
            | Self::Execution { step, .. } => Some(step),
        }
    }
}

/// A failed command whose message is its stderr, or `fallback` when it
/// printed nothing.
fn command_failure(
    executable: impl Into<String>,
    arguments: Vec<String>,
    exit_status: Option<i32>,
    stderr: impl Into<String>,
    fallback: &str,
) -> (FailedStep, String) {
    let stderr = stderr.into();
    let message = if stderr.trim().is_empty() {
        fallback.to_owned()
    } else {
        stderr.trim().to_owned()
    };
    (
        FailedStep::Command {
            executable: executable.into(),
            arguments,
            exit_status,
            stderr,
        },
        message,
    )
}
