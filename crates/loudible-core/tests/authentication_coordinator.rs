use std::sync::Arc;

use LoginCallbackError as L;
use loudible_core::*;
use loudible_test_support::*;

struct Fixture {
    api: Arc<MockAudibleApi>,
    store: Arc<InMemoryCredentialStore>,
    clock: Arc<TestClock>,
}

const US: Marketplace = Marketplace::US;

impl Fixture {
    fn new() -> Self {
        Self {
            api: Arc::default(),
            store: Arc::default(),
            clock: Arc::default(),
        }
    }

    fn coordinator(&self) -> AuthenticationCoordinator {
        AuthenticationCoordinator::new(self.api.clone(), self.store.clone(), self.clock.clone())
    }

    fn script_registration(&self) {
        self.api.on_register(|_, _| async { Ok(session()) });
    }

    fn restored(&self, session: AudibleSession) -> AuthenticationCoordinator {
        self.store.stored.set(Some(session));
        let auth = self.coordinator();
        auth.restore().unwrap();
        auth
    }

    /// Registration waits at the returned gate.
    fn hold_registration(&self) -> AsyncGate {
        let gate = AsyncGate::default();
        let held = gate.clone();
        self.api.on_register(move |_, _| {
            let held = held.clone();
            async move {
                held.wait().await;
                Ok(session())
            }
        });
        gate
    }

    /// Refreshing waits at the returned gate, then succeeds with the token "new".
    fn hold_refresh(&self) -> AsyncGate {
        let gate = AsyncGate::default();
        let held = gate.clone();
        self.api.on_refresh(move |_| {
            let held = held.clone();
            async move {
                held.wait().await;
                Ok(session_with_token("new"))
            }
        });
        gate
    }

    async fn wait_for_refresh(&self) {
        let api = self.api.clone();
        wait_until(move || api.refresh_count.current() == 1).await;
    }
}

async fn wait_for_registering(auth: &AuthenticationCoordinator) {
    let auth = auth.clone();
    wait_until(move || matches!(auth.state(), AuthState::Registering(_))).await;
}

fn callback_error(error: L) -> ExporterError {
    ExporterError::LoginCallback(error)
}

#[tokio::test]
async fn restores_stored_session() {
    let f = Fixture::new();
    f.store.stored.set(Some(session()));
    let auth = f.coordinator();
    assert_eq!(auth.restore().unwrap(), Some(session()));
    assert_eq!(auth.state(), AuthState::SignedIn(session()));
}

#[tokio::test]
async fn restore_without_credentials_is_signed_out() {
    let auth = Fixture::new().coordinator();
    assert_eq!(auth.restore().unwrap(), None);
    assert_eq!(auth.state(), AuthState::SignedOut);
}

#[tokio::test]
async fn restore_storage_error_is_reported_and_signed_out() {
    let f = Fixture::new();
    f.store.fail_load.set(true);
    let auth = f.coordinator();
    assert!(auth.restore().is_err());
    assert_eq!(auth.state(), AuthState::SignedOut);
}

#[tokio::test]
async fn completes_login_and_persists_session() {
    let f = Fixture::new();
    f.script_registration();
    let auth = f.coordinator();
    let login = auth.begin_login(US).unwrap();
    assert_eq!(auth.state(), AuthState::AwaitingCallback(login));
    let result = auth
        .complete_login(&callback_url(US, "the-code"))
        .await
        .unwrap();
    assert_eq!(result, session());
    assert_eq!(auth.state(), AuthState::SignedIn(result.clone()));
    assert_eq!(f.store.stored.current(), Some(result));
    assert_eq!(f.api.register_count.current(), 1);
}

#[tokio::test]
async fn passes_pending_verifier_and_code_to_registration() {
    let f = Fixture::new();
    let received = Arc::new(Locked::new(None));
    let record = received.clone();
    f.api.on_register(move |login, code| {
        record.set(Some((login, code)));
        async { Ok(session()) }
    });
    let auth = f.coordinator();
    let login = auth.begin_login(US).unwrap();
    auth.complete_login(&callback_url(US, "abc")).await.unwrap();
    assert_eq!(received.current(), Some((login, "abc".to_string())));
}

#[tokio::test]
async fn keeps_plus_signs_in_authorization_codes() {
    let f = Fixture::new();
    let received = Arc::new(Locked::new(String::new()));
    let record = received.clone();
    f.api.on_register(move |_, code| {
        record.set(code);
        async { Ok(session()) }
    });
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    auth.complete_login(&callback_url(US, "a+b/c"))
        .await
        .unwrap();
    assert_eq!(received.current(), "a+b/c");
}

#[tokio::test]
async fn keeps_reserved_characters_in_authorization_codes() {
    let f = Fixture::new();
    let received = Arc::new(Locked::new(String::new()));
    let record = received.clone();
    f.api.on_register(move |_, code| {
        record.set(code);
        async { Ok(session()) }
    });
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    auth.complete_login(&callback_url(US, "a+b/c%d&e=f"))
        .await
        .unwrap();
    assert_eq!(received.current(), "a+b/c%d&e=f");
}

#[tokio::test]
async fn rejects_duplicate_callback() {
    let f = Fixture::new();
    f.script_registration();
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    auth.complete_login(&us_callback()).await.unwrap();
    assert_eq!(
        auth.complete_login(&us_callback()).await,
        Err(callback_error(L::NoPendingLogin))
    );
    assert_eq!(f.api.register_count.current(), 1);
}

#[tokio::test]
async fn rejects_concurrent_duplicate_callback_while_registering() {
    let f = Fixture::new();
    let gate = f.hold_registration();
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    let first = {
        let auth = auth.clone();
        tokio::spawn(async move { auth.complete_login(&us_callback()).await })
    };
    wait_for_registering(&auth).await;
    assert_eq!(
        auth.complete_login(&us_callback()).await,
        Err(callback_error(L::NoPendingLogin))
    );
    gate.open();
    first.await.unwrap().unwrap();
    assert_eq!(f.api.register_count.current(), 1);
}

#[tokio::test]
async fn callback_without_login_is_rejected() {
    let auth = Fixture::new().coordinator();
    assert_eq!(
        auth.complete_login(&us_callback()).await,
        Err(callback_error(L::NoPendingLogin))
    );
}

#[tokio::test]
async fn callback_after_cancel_is_stale() {
    let auth = Fixture::new().coordinator();
    auth.begin_login(US).unwrap();
    auth.cancel_login();
    assert_eq!(auth.state(), AuthState::SignedOut);
    assert_eq!(
        auth.complete_login(&us_callback()).await,
        Err(callback_error(L::NoPendingLogin))
    );
}

#[tokio::test]
async fn expired_login_is_rejected() {
    let f = Fixture::new();
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    f.clock.advance(PendingLogin::LIFETIME_SECONDS + 1);
    assert_eq!(
        auth.complete_login(&us_callback()).await,
        Err(callback_error(L::Expired))
    );
    assert_eq!(auth.state(), AuthState::SignedOut);
    assert_eq!(f.api.register_count.current(), 0);
}

#[tokio::test]
async fn new_login_supersedes_old_one() {
    let f = Fixture::new();
    let ids = Arc::new(Locked::new(0u128));
    f.api.on_make_login(move |marketplace| {
        let id = ids.with(|n| {
            *n += 1;
            *n
        });
        Ok(pending_login(marketplace, login_id(id), reference_date()))
    });
    let received = Arc::new(Locked::new(None));
    let record = received.clone();
    f.api.on_register(move |login, _| {
        record.set(Some(login));
        async { Ok(session_in(Marketplace::UK)) }
    });
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    let second = auth.begin_login(Marketplace::UK).unwrap();

    // A US callback no longer matches the pending UK login.
    assert_eq!(
        auth.complete_login(&us_callback()).await,
        Err(callback_error(L::UnexpectedHost(Some(
            "www.amazon.com".into()
        ))))
    );
    auth.complete_login(&callback_url(Marketplace::UK, "c"))
        .await
        .unwrap();
    assert_eq!(received.current(), Some(second));
}

#[tokio::test]
async fn malformed_callback_keeps_login_pending() {
    const CODE: &str = "openid.oa2.authorization_code";
    let cases = [
        (
            format!("http://www.amazon.com/ap/maplanding?{CODE}=x"),
            L::MalformedUrl,
        ),
        (
            format!("https://www.amazon.evil.com/ap/maplanding?{CODE}=x"),
            L::UnexpectedHost(Some("www.amazon.evil.com".into())),
        ),
        (
            format!("https://www.amazon.com/ap/signin?{CODE}=x"),
            L::UnexpectedPath("/ap/signin".into()),
        ),
        (
            "https://www.amazon.com/ap/maplanding?other=1".to_string(),
            L::MissingAuthorizationCode,
        ),
        (
            format!("https://www.amazon.com/ap/maplanding?{CODE}="),
            L::MissingAuthorizationCode,
        ),
        (
            format!("https://www.amazon.com/ap/maplanding?{CODE}=a&{CODE}=b"),
            L::MissingAuthorizationCode,
        ),
        ("not a url at all".to_string(), L::MalformedUrl),
    ];
    for (url, expected) in cases {
        let f = Fixture::new();
        let auth = f.coordinator();
        let login = auth.begin_login(US).unwrap();
        assert_eq!(
            auth.complete_login(&url).await,
            Err(callback_error(expected)),
            "{url}"
        );
        assert_eq!(auth.state(), AuthState::AwaitingCallback(login));
        assert_eq!(f.api.register_count.current(), 0);
    }
}

#[test]
fn recognises_callback_for_browser_interception() {
    let login = pending_login(Marketplace::JAPAN, login_id(1), reference_date());
    assert!(LoginCallback::is_callback(
        "https://www.amazon.co.jp/ap/maplanding?x=1",
        &login
    ));
    assert!(!LoginCallback::is_callback(
        "https://www.amazon.co.jp/ap/signin",
        &login
    ));
    assert!(!LoginCallback::is_callback(
        "https://www.amazon.com/ap/maplanding",
        &login
    ));
}

#[tokio::test]
async fn registration_failure_requires_new_login() {
    let f = Fixture::new();
    let failure = ExporterError::Http {
        status: 400,
        message: Some("bad code".into()),
    };
    let thrown = failure.clone();
    f.api.on_register(move |_, _| {
        let thrown = thrown.clone();
        async move { Err(thrown) }
    });
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    assert_eq!(auth.complete_login(&us_callback()).await, Err(failure));
    assert_eq!(auth.state(), AuthState::SignedOut);
    assert_eq!(f.store.stored.current(), None);
}

#[tokio::test]
async fn cancelling_during_registration_discards_result() {
    let f = Fixture::new();
    let gate = f.hold_registration();
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    let completion = {
        let auth = auth.clone();
        tokio::spawn(async move { auth.complete_login(&us_callback()).await })
    };
    wait_for_registering(&auth).await;
    auth.cancel_login();
    gate.open();
    assert_eq!(completion.await.unwrap(), Err(ExporterError::Cancelled));
    assert_eq!(auth.state(), AuthState::SignedOut);
    assert_eq!(f.store.stored.current(), None);
}

#[tokio::test]
async fn save_failure_keeps_session_with_warning() {
    let f = Fixture::new();
    f.script_registration();
    f.store.fail_save.set(true);
    let auth = f.coordinator();
    auth.begin_login(US).unwrap();
    auth.complete_login(&us_callback()).await.unwrap();
    assert!(auth.state().session().is_some());
    assert!(auth.storage_warning().is_some());
}

#[tokio::test]
async fn valid_session_without_sign_in_requires_authentication() {
    let auth = Fixture::new().coordinator();
    assert_eq!(
        auth.valid_session().await,
        Err(ExporterError::AuthenticationRequired)
    );
}

#[tokio::test]
async fn fresh_token_is_returned_without_refresh() {
    let f = Fixture::new();
    let auth = f.restored(session());
    assert_eq!(auth.valid_session().await.unwrap(), session());
    assert_eq!(f.api.refresh_count.current(), 0);
}

#[tokio::test]
async fn expired_token_is_refreshed_and_persisted() {
    let f = Fixture::new();
    let expired = AudibleSession {
        access_token_expires_at: seconds_after_reference(-10),
        ..session()
    };
    let refreshed = session_with_token("new");
    let reply = refreshed.clone();
    f.api.on_refresh(move |_| {
        let reply = reply.clone();
        async move { Ok(reply) }
    });
    let auth = f.restored(expired);
    assert_eq!(auth.valid_session().await.unwrap(), refreshed);
    assert_eq!(auth.state(), AuthState::SignedIn(refreshed.clone()));
    assert_eq!(f.store.stored.current(), Some(refreshed));
}

#[tokio::test]
async fn concurrent_callers_share_one_refresh() {
    let f = Fixture::new();
    let gate = f.hold_refresh();
    let auth = f.restored(expired_session());
    let callers: Vec<_> = (0..20)
        .map(|_| {
            let auth = auth.clone();
            tokio::spawn(async move { auth.valid_session().await })
        })
        .collect();
    f.wait_for_refresh().await;
    gate.open();
    for caller in callers {
        assert_eq!(caller.await.unwrap().unwrap().access_token, "new");
    }
    assert_eq!(f.api.refresh_count.current(), 1);
}

#[tokio::test]
async fn rejected_refresh_requires_reauthentication_and_clears_credentials() {
    let f = Fixture::new();
    f.api
        .on_refresh(|_| async { Err(ExporterError::AuthenticationRequired) });
    let auth = f.restored(expired_session());
    assert_eq!(
        auth.valid_session().await,
        Err(ExporterError::AuthenticationRequired)
    );
    assert_eq!(auth.state(), AuthState::ReauthenticationRequired(US));
    assert_eq!(f.store.stored.current(), None);
}

#[tokio::test]
async fn transient_refresh_failure_keeps_session() {
    let f = Fixture::new();
    f.api
        .on_refresh(|_| async { Err(ExporterError::Network("timed out".into())) });
    let auth = f.restored(expired_session());
    assert!(auth.valid_session().await.is_err());
    assert!(auth.state().session().is_some());
    assert!(f.store.stored.current().is_some());
}

#[tokio::test]
async fn invalidated_token_is_refreshed_next_time() {
    let f = Fixture::new();
    f.api
        .on_refresh(|_| async { Ok(session_with_token("new")) });
    let auth = f.restored(session());
    auth.invalidate_access_token();
    assert_eq!(auth.valid_session().await.unwrap().access_token, "new");
    assert_eq!(f.api.refresh_count.current(), 1);
}

#[tokio::test]
async fn sign_out_during_refresh_does_not_resurrect_session() {
    let f = Fixture::new();
    let gate = f.hold_refresh();
    let auth = f.restored(expired_session());
    let caller = {
        let auth = auth.clone();
        tokio::spawn(async move { auth.valid_session().await })
    };
    f.wait_for_refresh().await;
    auth.sign_out().unwrap();
    gate.open();
    assert_eq!(
        caller.await.unwrap(),
        Err(ExporterError::AuthenticationRequired)
    );
    assert_eq!(auth.state(), AuthState::SignedOut);
    assert_eq!(f.store.stored.current(), None);
}

#[tokio::test]
async fn sign_out_removes_local_credentials() {
    let f = Fixture::new();
    let auth = f.restored(session());
    auth.sign_out().unwrap();
    assert_eq!(auth.state(), AuthState::SignedOut);
    assert_eq!(f.store.stored.current(), None);
}

#[tokio::test]
async fn sign_out_storage_error_is_reported_but_signs_out() {
    let f = Fixture::new();
    let auth = f.restored(session());
    f.store.fail_delete.set(true);
    assert!(auth.sign_out().is_err());
    assert_eq!(auth.state(), AuthState::SignedOut);
}

#[tokio::test]
async fn publishes_state_updates() {
    let f = Fixture::new();
    f.script_registration();
    let auth = f.coordinator();
    let mut updates = auth.state_updates();
    let login = auth.begin_login(US).unwrap();
    auth.complete_login(&us_callback()).await.unwrap();
    let mut states = Vec::new();
    while let Some(state) = updates.recv().await {
        let is_done = state.session().is_some();
        states.push(state);
        if is_done {
            break;
        }
    }
    assert_eq!(
        states,
        [
            AuthState::SignedOut,
            AuthState::AwaitingCallback(login.clone()),
            AuthState::Registering(login),
            AuthState::SignedIn(session()),
        ]
    );
}
