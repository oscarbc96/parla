use serde::Deserialize;
use url::Url;

use crate::{ApiError, WebClient, method};

/// Response of `auth.test`. Unknown fields are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthTest {
    pub url: Url,
    pub team: String,
    pub user: String,
    pub team_id: String,
    pub user_id: String,
    #[serde(default)]
    pub enterprise_id: Option<String>,
    #[serde(default)]
    pub is_enterprise_install: bool,
}

impl WebClient {
    pub async fn auth_test(&self) -> Result<AuthTest, ApiError> {
        self.call(method::AUTH_TEST, &[], crate::Priority::User)
            .await
    }
}
