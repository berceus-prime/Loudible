//! Ported from `SessionViewModelTests.swift` and its storage extension.

mod support;

use std::rc::Rc;
use std::sync::Arc;

use loudible_core::*;
use loudible_presentation::*;
use loudible_test_support::*;
use support::*;

struct Setup {
    api: Arc<MockAudibleApi>,
    store: Arc<InMemoryCredentialStore>,
    errors: Rc<ErrorToastQueue>,
}

impl Setup {
    fn new() -> Self {
        Self {
            api: Arc::default(),
            store: Arc::default(),
            errors: Rc::default(),
        }
    }

    fn auth(&self) -> AuthenticationCoordinator {
        AuthenticationCoordinator::new(
            self.api.clone(),
            self.store.clone(),
            Arc::new(TestClock::default()),
        )
    }

    fn model(&self, auth: AuthenticationCoordinator) -> SessionViewModel {
        let model = SessionViewModel::new(auth, Changes::default());
        let errors = self.errors.clone();
        model.set_on_error(move |error| errors.present(error));
        model
    }

    async fn started(&self) -> (SessionViewModel, AuthenticationCoordinator) {
        let auth = self.auth();
        let model = self.model(auth.clone());
        model.start().await;
        (model, auth)
    }

    fn registers(&self) {
        self.api.on_register(|_, _| async { Ok(session()) });
    }
}

fn expiring() -> Option<AudibleSession> {
    Some(expired_session())
}

#[test]
fn signed_out_state_enables_sign_in_only() {
    local(async {
        let (model, _) = Setup::new().started().await;
        assert!(!model.is_signed_in());
        assert!(model.can_sign_in());
        assert!(model.can_change_marketplace());
        assert!(!model.can_sign_out());
        assert!(model.pending_login().is_none());
    });
}

#[test]
fn restores_saved_session_and_marketplace() {
    local(async {
        let setup = Setup::new();
        setup.store.stored.set(Some(session_in(Marketplace::JAPAN)));
        let (model, _) = setup.started().await;
        assert!(model.is_signed_in());
        assert_eq!(model.marketplace(), Marketplace::JAPAN);
        assert!(model.can_sign_out() && !model.can_sign_in());
        assert_eq!(model.account_description().unwrap(), "Test User · Japan");
    });
}

#[test]
fn storage_failure_on_restore_is_shown() {
    local(async {
        let setup = Setup::new();
        setup.store.fail_load.set(true);
        let (model, _) = setup.started().await;
        let notices = setup.errors.notices();
        assert_eq!(notices.len(), 1);
        let AppError::CredentialStorage(failure) = &notices[0].error else {
            panic!("expected a credential-storage toast");
        };
        assert_eq!(failure.operation, CredentialOperation::Load);
        assert!(!failure.technical_error.is_empty());
        assert!(!model.is_signed_in());
        assert!(model.has_restored());
    });
}

#[test]
fn has_restored_only_after_start() {
    local(async {
        let setup = Setup::new();
        let model = setup.model(setup.auth());
        assert!(!model.has_restored());
        model.start().await;
        assert!(model.has_restored());
    });
}

#[test]
fn sign_in_flow_with_browser_callback() {
    local(async {
        let setup = Setup::new();
        setup
            .api
            .on_register(|login, _| async move { Ok(session_in(login.marketplace)) });
        let (model, _) = setup.started().await;
        model.set_marketplace(Marketplace::UK);
        model.sign_in();
        assert_eq!(model.pending_login().unwrap().marketplace, Marketplace::UK);
        assert!(!model.can_change_marketplace());
        let callback = callback_url(Marketplace::UK, "synthetic-code");
        assert!(model.is_callback(&callback));
        assert!(!model.is_callback("https://www.amazon.co.uk/ap/signin"));

        model.complete_sign_in(&callback).await;
        assert!(model.is_signed_in());
        assert!(model.pending_login().is_none());
        assert!(model.error_message().is_none());
    });
}

#[test]
fn pasted_url_fallback_uses_same_coordinator() {
    local(async {
        let setup = Setup::new();
        setup.registers();
        let (model, _) = setup.started().await;
        model.sign_in();
        assert!(!model.can_submit_pasted_url());

        model.set_pasted_callback_url("https://www.amazon.com/ap/maplanding?nothing=here");
        assert!(model.can_submit_pasted_url());
        model.submit_pasted_url().await;
        assert!(model.error_message().is_some());
        assert!(model.pending_login().is_some()); // still waiting for a usable URL

        model.set_pasted_callback_url(format!("  {}\n", us_callback()));
        model.submit_pasted_url().await;
        assert!(model.is_signed_in());
        assert!(model.pasted_callback_url().is_empty());
    });
}

#[test]
fn an_unparsable_pasted_url_is_reported() {
    local(async {
        let setup = Setup::new();
        let (model, _) = setup.started().await;
        model.sign_in();
        model.set_pasted_callback_url("not a url");
        model.submit_pasted_url().await;
        assert_eq!(
            model.error_message().unwrap(),
            ExporterError::LoginCallback(LoginCallbackError::MalformedUrl).to_string()
        );
    });
}

#[test]
fn cancel_sign_in_returns_to_signed_out() {
    local(async {
        let (model, _) = Setup::new().started().await;
        model.sign_in();
        model.cancel_sign_in();
        assert!(model.pending_login().is_none());
        assert!(model.can_sign_in());
    });
}

#[test]
fn sign_out_clears_session() {
    local(async {
        let setup = Setup::new();
        setup.store.stored.set(Some(session()));
        let (model, _) = setup.started().await;
        model.sign_out();
        assert!(!model.is_signed_in());
        assert_eq!(setup.store.stored.current(), None);
    });
}

#[test]
fn reflects_reauthentication_requirement() {
    local(async {
        let setup = Setup::new();
        setup.store.stored.set(expiring());
        setup
            .api
            .on_refresh(|_| async { Err(ExporterError::AuthenticationRequired) });
        let (model, auth) = setup.started().await;
        let _ = auth.valid_session().await;
        wait_until(|| model.needs_reauthentication()).await;
        assert!(model.can_sign_in());
        assert!(model.can_sign_out());
    });
}

#[test]
fn save_failure_after_sign_in_is_toasted_once() {
    local(async {
        let setup = Setup::new();
        setup.store.fail_save.set(true);
        setup.registers();
        let (model, _) = setup.started().await;
        model.sign_in();
        model.complete_sign_in(&us_callback()).await;
        wait_until(|| setup.errors.notices().len() == 1).await;
        assert!(model.is_signed_in());
        assert!(model.error_message().is_none());
        assert_eq!(
            setup.errors.notices()[0].error.title(),
            "Couldn’t save your sign-in"
        );
    });
}

#[test]
fn refresh_storage_warnings_reach_toasts() {
    local(async {
        let setup = Setup::new();
        setup.store.stored.set(expiring());
        setup.store.fail_save.set(true);
        setup
            .api
            .on_refresh(|_| async { Ok(session_with_token("refreshed")) });
        let (model, auth) = setup.started().await;
        auth.valid_session().await.unwrap();
        wait_until(|| setup.errors.notices().len() == 1).await;
        assert_eq!(
            setup.errors.notices()[0].error.title(),
            "Couldn’t save your sign-in"
        );
        assert!(model.is_signed_in());
    });
}

#[test]
fn deletion_failures_are_toasted() {
    for expired in [false, true] {
        local(async {
            let setup = Setup::new();
            setup
                .store
                .stored
                .set(if expired { expiring() } else { Some(session()) });
            setup.store.fail_delete.set(true);
            setup
                .api
                .on_refresh(|_| async { Err(ExporterError::AuthenticationRequired) });
            let (model, auth) = setup.started().await;
            if expired {
                let _ = auth.valid_session().await;
            } else {
                model.sign_out();
            }
            wait_until(|| setup.errors.notices().len() == 1).await;
            let error = &setup.errors.notices()[0].error;
            assert_eq!(error.title(), "Couldn’t remove saved credentials");
            assert!(!error.technical_error().is_empty());
        });
    }
}
