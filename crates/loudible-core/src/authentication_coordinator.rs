use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::DateTime;
use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::{
    AudibleApi, AudibleSession, CredentialOperation, CredentialStorageFailure, CredentialStore,
    ExporterError, LoginCallback, LoginCallbackError, Marketplace, PendingLogin, WallClock,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthState {
    SignedOut,
    /// The browser is showing the login page for this attempt.
    AwaitingCallback(PendingLogin),
    /// The callback was accepted and the device is being registered.
    Registering(PendingLogin),
    SignedIn(AudibleSession),
    /// Stored credentials stopped working; the user must sign in again.
    ReauthenticationRequired(Marketplace),
}

impl AuthState {
    pub fn session(&self) -> Option<&AudibleSession> {
        match self {
            Self::SignedIn(session) => Some(session),
            _ => None,
        }
    }
}

type RefreshResult = Result<AudibleSession, ExporterError>;

struct Refresh {
    id: u64,
    session: AudibleSession,
    result: Shared<BoxFuture<'static, RefreshResult>>,
    abort: AbortHandle,
}

struct Core {
    state: AuthState,
    storage_warning: Option<CredentialStorageFailure>,
    refresh: Option<Refresh>,
    next_refresh_id: u64,
    observers: Vec<mpsc::UnboundedSender<AuthState>>,
    storage_observers: Vec<mpsc::UnboundedSender<CredentialStorageFailure>>,
}

impl Core {
    fn set_state(&mut self, state: AuthState) {
        if self.state != state {
            self.state = state;
            let current = &self.state;
            self.observers
                .retain(|observer| observer.send(current.clone()).is_ok());
        }
    }

    fn set_warning(&mut self, warning: Option<CredentialStorageFailure>) {
        if let Some(failure) = &warning {
            self.storage_observers
                .retain(|observer| observer.send(failure.clone()).is_ok());
        }
        self.storage_warning = warning;
    }
}

struct Inner {
    api: Arc<dyn AudibleApi>,
    store: Arc<dyn CredentialStore>,
    clock: Arc<dyn WallClock>,
    core: Mutex<Core>,
}

/// Owns the sign-in lifecycle: starting and validating logins, restoring and persisting the
/// session, refreshing access tokens (one refresh at a time), and local sign-out.
///
/// The state is only locked between awaits, so calls interleave as they would on an actor.
#[derive(Clone)]
pub struct AuthenticationCoordinator(Arc<Inner>);

impl AuthenticationCoordinator {
    pub fn new(
        api: Arc<dyn AudibleApi>,
        store: Arc<dyn CredentialStore>,
        clock: Arc<dyn WallClock>,
    ) -> Self {
        Self(Arc::new(Inner {
            api,
            store,
            clock,
            core: Mutex::new(Core {
                state: AuthState::SignedOut,
                storage_warning: None,
                refresh: None,
                next_refresh_id: 0,
                observers: Vec::new(),
                storage_observers: Vec::new(),
            }),
        }))
    }

    fn core(&self) -> MutexGuard<'_, Core> {
        self.0.core.lock().expect("no panic while holding the lock")
    }

    pub fn state(&self) -> AuthState {
        self.core().state.clone()
    }

    /// Set when the session works but could not be saved or removed from storage.
    pub fn storage_warning(&self) -> Option<CredentialStorageFailure> {
        self.core().storage_warning.clone()
    }

    /// Emits the current state, then every change.
    pub fn state_updates(&self) -> mpsc::UnboundedReceiver<AuthState> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut core = self.core();
        let _ = sender.send(core.state.clone());
        core.observers.push(sender);
        receiver
    }

    /// Emits storage warnings independently of session changes, including token-refresh writes.
    pub fn storage_warnings(&self) -> mpsc::UnboundedReceiver<CredentialStorageFailure> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.core().storage_observers.push(sender);
        receiver
    }

    // MARK: Restoration

    /// Loads stored credentials. Returns the restored session, or `None` when there is none.
    pub fn restore(&self) -> Result<Option<AudibleSession>, ExporterError> {
        let mut core = self.core();
        match self.0.store.load() {
            Ok(Some(session)) => {
                core.set_state(AuthState::SignedIn(session.clone()));
                Ok(Some(session))
            }
            Ok(None) => {
                core.set_state(AuthState::SignedOut);
                Ok(None)
            }
            Err(error) => {
                core.set_state(AuthState::SignedOut);
                Err(ExporterError::CredentialStorage(error))
            }
        }
    }

    // MARK: Login

    /// Starts a new sign-in. Any earlier pending sign-in becomes stale.
    pub fn begin_login(&self, marketplace: Marketplace) -> Result<PendingLogin, ExporterError> {
        let login = self.0.api.make_login(marketplace)?;
        self.core()
            .set_state(AuthState::AwaitingCallback(login.clone()));
        Ok(login)
    }

    pub fn cancel_login(&self) {
        let mut core = self.core();
        if matches!(
            core.state,
            AuthState::AwaitingCallback(_) | AuthState::Registering(_)
        ) {
            core.set_state(AuthState::SignedOut);
        }
    }

    /// Validates the redirect URL and registers the device.
    ///
    /// A malformed URL leaves the sign-in pending so a corrected URL can be pasted. Once a
    /// callback is accepted, further callbacks are rejected as duplicates.
    pub async fn complete_login(
        &self,
        callback_url: &str,
    ) -> Result<AudibleSession, ExporterError> {
        let (login, code) = {
            let mut core = self.core();
            let AuthState::AwaitingCallback(login) = core.state.clone() else {
                return Err(ExporterError::LoginCallback(
                    LoginCallbackError::NoPendingLogin,
                ));
            };
            if login.is_expired(self.0.clock.now()) {
                core.set_state(AuthState::SignedOut);
                return Err(ExporterError::LoginCallback(LoginCallbackError::Expired));
            }
            let code = LoginCallback::authorization_code(callback_url, &login)
                .map_err(ExporterError::LoginCallback)?;
            core.set_state(AuthState::Registering(login.clone()));
            (login, code)
        };

        let result = self.0.api.register(&login, &code).await;
        let mut core = self.core();
        let is_current = core.state == AuthState::Registering(login);
        let session = match result {
            Ok(session) => session,
            Err(error) => {
                // The authorization code is single-use, so a failed registration needs a new login.
                if is_current {
                    core.set_state(AuthState::SignedOut);
                }
                return Err(error);
            }
        };
        if !is_current {
            // Cancelled or superseded while registering: discard the result.
            return Err(ExporterError::Cancelled);
        }
        core.set_state(AuthState::SignedIn(session.clone()));
        self.persist(&mut core, &session);
        Ok(session)
    }

    // MARK: Session use

    /// Returns a session whose access token is valid, refreshing it if necessary.
    /// Concurrent callers share a single refresh.
    pub async fn valid_session(&self) -> Result<AudibleSession, ExporterError> {
        let (session, refresh_id, result) = {
            let mut core = self.core();
            let Some(session) = core.state.session().cloned() else {
                return Err(ExporterError::AuthenticationRequired);
            };
            if !session.needs_refresh(self.0.clock.now()) {
                return Ok(session);
            }
            let existing = core
                .refresh
                .as_ref()
                .filter(|refresh| refresh.session.device == session.device)
                .map(|refresh| (refresh.id, refresh.result.clone()));
            let (id, result) = existing.unwrap_or_else(|| self.start_refresh(&mut core, &session));
            (session, id, result)
        };

        let result = result.await;

        let mut core = self.core();
        if core.refresh.as_ref().is_some_and(|r| r.id == refresh_id) {
            core.refresh = None;
        }
        // Signed out, or signed in as another device, while refreshing.
        let Some(current) = core
            .state
            .session()
            .filter(|current| current.device == session.device)
            .cloned()
        else {
            return Err(ExporterError::AuthenticationRequired);
        };
        match result {
            Ok(refreshed) => {
                if current != refreshed {
                    core.set_state(AuthState::SignedIn(refreshed.clone()));
                    self.persist(&mut core, &refreshed);
                }
                Ok(refreshed)
            }
            Err(error) => {
                if error == ExporterError::AuthenticationRequired {
                    core.set_state(AuthState::ReauthenticationRequired(session.marketplace));
                    self.delete_stored_credentials(&mut core);
                }
                Err(error)
            }
        }
    }

    /// Runs apart from any caller, so a cancelled job can't take other callers' refresh with it.
    fn start_refresh(
        &self,
        core: &mut Core,
        session: &AudibleSession,
    ) -> (u64, Shared<BoxFuture<'static, RefreshResult>>) {
        let api = self.0.api.clone();
        let refreshing = session.clone();
        let task = tokio::spawn(async move { api.refresh(&refreshing).await });
        let abort = task.abort_handle();
        let result = async move { task.await.unwrap_or(Err(ExporterError::Cancelled)) }
            .boxed()
            .shared();
        core.next_refresh_id += 1;
        let id = core.next_refresh_id;
        core.refresh = Some(Refresh {
            id,
            session: session.clone(),
            result: result.clone(),
            abort,
        });
        (id, result)
    }

    /// Runs an API call with a valid session. If Audible rejects the credentials, the token
    /// is refreshed and the call retried once; a second rejection requires signing in again.
    pub async fn with_valid_session<T, F, Fut>(&self, operation: F) -> Result<T, ExporterError>
    where
        F: Fn(AudibleSession) -> Fut,
        Fut: Future<Output = Result<T, ExporterError>>,
    {
        for attempt in 1..=2 {
            let session = self.valid_session().await?;
            let device = session.device.clone();
            let marketplace = session.marketplace;
            match operation(session).await {
                Err(ExporterError::AuthenticationRequired) => {
                    let mut core = self.core();
                    // Signed out or signed in again meanwhile: leave the new state alone.
                    if core.state.session().map(|s| &s.device) != Some(&device) {
                        return Err(ExporterError::AuthenticationRequired);
                    }
                    if attempt == 2 {
                        core.set_state(AuthState::ReauthenticationRequired(marketplace));
                        self.delete_stored_credentials(&mut core);
                        return Err(ExporterError::AuthenticationRequired);
                    }
                    Self::invalidate(&mut core);
                }
                result => return result,
            }
        }
        Err(ExporterError::AuthenticationRequired)
    }

    /// Forces the next `valid_session()` to refresh, e.g. after the API rejected the token.
    pub fn invalidate_access_token(&self) {
        Self::invalidate(&mut self.core());
    }

    fn invalidate(core: &mut Core) {
        if let Some(mut session) = core.state.session().cloned() {
            session.access_token_expires_at = DateTime::UNIX_EPOCH;
            core.set_state(AuthState::SignedIn(session));
        }
    }

    /// Forgets the session locally. The device registration is not revoked with Audible.
    pub fn sign_out(&self) -> Result<(), ExporterError> {
        let mut core = self.core();
        if let Some(refresh) = core.refresh.take() {
            refresh.abort.abort();
        }
        core.set_state(AuthState::SignedOut);
        core.set_warning(None);
        self.0
            .store
            .delete()
            .map_err(ExporterError::CredentialStorage)
    }

    // MARK: Helpers

    fn persist(&self, core: &mut Core, session: &AudibleSession) {
        match self.0.store.save(session) {
            Ok(()) => core.set_warning(None),
            Err(technical_error) => core.set_warning(Some(CredentialStorageFailure {
                operation: CredentialOperation::Save,
                technical_error,
            })),
        }
    }

    fn delete_stored_credentials(&self, core: &mut Core) {
        if let Err(technical_error) = self.0.store.delete() {
            core.set_warning(Some(CredentialStorageFailure {
                operation: CredentialOperation::Delete,
                technical_error,
            }));
        }
    }
}
