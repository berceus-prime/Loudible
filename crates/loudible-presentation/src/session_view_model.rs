use std::cell::RefCell;
use std::rc::{Rc, Weak};

use loudible_core::{
    AudibleSession, AuthState, AuthenticationCoordinator, CredentialOperation,
    CredentialStorageFailure, LoginCallback, Marketplace, PendingLogin,
};

use crate::{AppError, Changes};

pub(crate) type ErrorSink = Rc<dyn Fn(AppError)>;

struct State {
    marketplace: Marketplace,
    auth_state: AuthState,
    pending_login: Option<PendingLogin>,
    pasted_callback_url: String,
    is_working: bool,
    error_message: Option<String>,
    has_restored: bool,
    is_observing: bool,
    on_error: Option<ErrorSink>,
}

struct Inner {
    auth: AuthenticationCoordinator,
    state: RefCell<State>,
    changes: Changes,
}

/// Sign-in state and actions for the window's account controls and login sheet.
#[derive(Clone)]
pub struct SessionViewModel(Rc<Inner>);

impl SessionViewModel {
    pub fn new(auth: AuthenticationCoordinator, changes: Changes) -> Self {
        Self(Rc::new(Inner {
            auth,
            changes,
            state: RefCell::new(State {
                marketplace: Marketplace::US,
                auth_state: AuthState::SignedOut,
                pending_login: None,
                pasted_callback_url: String::new(),
                is_working: false,
                error_message: None,
                has_restored: false,
                is_observing: false,
                on_error: None,
            }),
        }))
    }

    fn read<R>(&self, body: impl FnOnce(&State) -> R) -> R {
        body(&self.0.state.borrow())
    }

    fn update(&self, body: impl FnOnce(&mut State)) {
        body(&mut self.0.state.borrow_mut());
        self.0.changes.mark();
    }

    pub fn set_on_error(&self, sink: impl Fn(AppError) + 'static) {
        self.0.state.borrow_mut().on_error = Some(Rc::new(sink));
    }

    fn report(&self, error: AppError) {
        let sink = self.read(|state| state.on_error.clone());
        if let Some(sink) = sink {
            sink(error);
        }
    }

    /// Restores saved credentials and starts mirroring the coordinator's state.
    pub async fn start(&self) {
        self.observe();
        match self.0.auth.restore() {
            Ok(Some(session)) => self.update(|state| state.marketplace = session.marketplace),
            Ok(None) => {}
            Err(error) => self.report(AppError::CredentialStorage(CredentialStorageFailure {
                operation: CredentialOperation::Load,
                technical_error: error.to_string(),
            })),
        }
        self.apply_current();
        self.update(|state| state.has_restored = true);
    }

    fn observe(&self) {
        if std::mem::replace(&mut self.0.state.borrow_mut().is_observing, true) {
            return;
        }
        let upgrade = |weak: &Weak<Inner>| weak.upgrade().map(Self);
        let weak = Rc::downgrade(&self.0);
        let mut warnings = self.0.auth.storage_warnings();
        tokio::task::spawn_local(async move {
            while let Some(warning) = warnings.recv().await {
                let Some(model) = upgrade(&weak) else { break };
                model.report(AppError::CredentialStorage(warning));
            }
        });
        let weak = Rc::downgrade(&self.0);
        let mut updates = self.0.auth.state_updates();
        tokio::task::spawn_local(async move {
            while let Some(state) = updates.recv().await {
                let Some(model) = upgrade(&weak) else { break };
                model.apply(state);
            }
        });
    }

    fn apply_current(&self) {
        self.apply(self.0.auth.state());
    }

    fn apply(&self, new: AuthState) {
        let changed = self.read(|state| state.auth_state != new);
        self.update(|state| {
            state.pending_login = match &new {
                AuthState::AwaitingCallback(login) => Some(login.clone()),
                _ => None,
            };
            match &new {
                AuthState::SignedIn(session) => state.marketplace = session.marketplace,
                AuthState::ReauthenticationRequired(marketplace) => {
                    state.marketplace = *marketplace;
                }
                _ => {}
            }
            state.auth_state = new;
        });
        if changed {
            self.0.changes.mark_session();
        }
    }

    // MARK: Derived state

    pub fn marketplace(&self) -> Marketplace {
        self.read(|state| state.marketplace)
    }

    pub fn set_marketplace(&self, marketplace: Marketplace) {
        self.update(|state| state.marketplace = marketplace);
    }

    pub fn pending_login(&self) -> Option<PendingLogin> {
        self.read(|state| state.pending_login.clone())
    }

    pub fn pasted_callback_url(&self) -> String {
        self.read(|state| state.pasted_callback_url.clone())
    }

    pub fn set_pasted_callback_url(&self, text: impl Into<String>) {
        let text = text.into();
        self.update(|state| state.pasted_callback_url = text);
    }

    pub fn is_working(&self) -> bool {
        self.read(|state| state.is_working)
    }

    pub fn error_message(&self) -> Option<String> {
        self.read(|state| state.error_message.clone())
    }

    pub fn has_restored(&self) -> bool {
        self.read(|state| state.has_restored)
    }

    pub fn session(&self) -> Option<AudibleSession> {
        self.read(|state| state.auth_state.session().cloned())
    }

    pub fn is_signed_in(&self) -> bool {
        self.read(|state| state.auth_state.session().is_some())
    }

    pub fn needs_reauthentication(&self) -> bool {
        self.read(|state| matches!(state.auth_state, AuthState::ReauthenticationRequired(_)))
    }

    pub fn is_registering(&self) -> bool {
        self.read(|state| matches!(state.auth_state, AuthState::Registering(_)))
    }

    pub fn can_sign_in(&self) -> bool {
        !self.is_working() && !self.is_signed_in() && !self.is_registering()
    }

    pub fn can_sign_out(&self) -> bool {
        !self.is_working() && (self.is_signed_in() || self.needs_reauthentication())
    }

    pub fn can_change_marketplace(&self) -> bool {
        self.can_sign_in() && self.pending_login().is_none()
    }

    pub fn can_submit_pasted_url(&self) -> bool {
        self.pending_login().is_some()
            && !self.is_working()
            && !self.pasted_callback_url().trim().is_empty()
    }

    pub fn account_description(&self) -> Option<String> {
        self.session().map(|session| {
            format!(
                "{} · {}",
                session.customer_name.as_deref().unwrap_or("Signed in"),
                session.marketplace.name
            )
        })
    }

    // MARK: Actions

    pub fn sign_in(&self) {
        self.update(|state| {
            state.error_message = None;
            state.pasted_callback_url.clear();
        });
        match self.0.auth.begin_login(self.marketplace()) {
            Ok(login) => self.update(|state| state.pending_login = Some(login)),
            Err(error) => self.update(|state| state.error_message = Some(error.to_string())),
        }
    }

    /// True when the embedded browser should stop and hand this URL to `complete_sign_in`.
    pub fn is_callback(&self, url: &str) -> bool {
        self.pending_login()
            .is_some_and(|login| LoginCallback::is_callback(url, &login))
    }

    pub async fn complete_sign_in(&self, url: &str) {
        self.update(|state| {
            state.is_working = true;
            state.error_message = None;
        });
        match self.0.auth.complete_login(url).await {
            Ok(_) => self.update(|state| state.pasted_callback_url.clear()),
            Err(error) => self.update(|state| state.error_message = Some(error.to_string())),
        }
        self.apply_current();
        self.update(|state| state.is_working = false);
    }

    pub async fn submit_pasted_url(&self) {
        let text = self.pasted_callback_url().trim().to_string();
        self.complete_sign_in(&text).await;
    }

    pub fn cancel_sign_in(&self) {
        self.0.auth.cancel_login();
        self.apply_current();
    }

    pub fn sign_out(&self) {
        self.update(|state| state.error_message = None);
        if let Err(error) = self.0.auth.sign_out() {
            self.report(AppError::CredentialStorage(CredentialStorageFailure {
                operation: CredentialOperation::Delete,
                technical_error: error.to_string(),
            }));
        }
        self.apply_current();
    }

    /// Called by the embedded browser after excluding callback redirects and cancelled navigation.
    pub fn browser_failed(&self, technical_error: impl Into<String>) {
        self.report(AppError::SignInBrowser {
            technical_error: technical_error.into(),
        });
    }
}
