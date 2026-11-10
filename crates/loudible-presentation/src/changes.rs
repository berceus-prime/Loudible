use std::rc::Rc;

use tokio::sync::Notify;

/// Tells waiters that a model changed. A signal holds until awaited, so a change made while
/// nobody is waiting is not lost, and several changes collapse into one wake-up.
#[derive(Clone, Default)]
pub struct Changes(Rc<Signals>);

#[derive(Default)]
struct Signals {
    ui: Notify,
    session: Notify,
}

impl Changes {
    /// Something on screen may have changed.
    pub fn mark(&self) {
        self.0.ui.notify_one();
    }

    /// The sign-in state, or whether jobs wait for it, changed.
    pub fn mark_session(&self) {
        self.mark();
        self.0.session.notify_one();
    }

    pub async fn ui_changed(&self) {
        self.0.ui.notified().await;
    }

    pub async fn session_changed(&self) {
        self.0.session.notified().await;
    }
}
