use std::sync::Arc;

use crate::{AudibleApi, Audiobook, AuthenticationCoordinator, ContentLicense, ExporterError};

/// Obtains a license through the current authenticated session.
#[derive(Clone)]
pub struct LicenseProvider {
    pub auth: AuthenticationCoordinator,
    pub api: Arc<dyn AudibleApi>,
}

impl LicenseProvider {
    /// The license and the account it belongs to.
    pub async fn fetch(&self, book: &Audiobook) -> Result<(ContentLicense, String), ExporterError> {
        self.auth
            .with_valid_session(|session| async move {
                let license = self
                    .api
                    .license(&book.asin, book.marketplace, &session)
                    .await?;
                Ok((license, session.customer_id))
            })
            .await
    }
}
