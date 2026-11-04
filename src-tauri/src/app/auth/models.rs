use super::*;

pub(crate) const DEFAULT_MS_CLIENT_ID: &str = "ab9b8c07-8f02-4f72-87fa-80105867a763";
pub(crate) const DEFAULT_INTERACTIVE_CLIENT_ID: &str = "d50ca740-c83f-4d1b-b616-12c519384f0c";
pub(crate) const DEFAULT_SCOPE: &str =
    "offline_access Files.ReadWrite.All User.Read openid profile email";
pub(crate) const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/devicecode";
pub(crate) const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
pub(crate) const INTERACTIVE_AUTHORIZE_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
pub(crate) const INTERACTIVE_TOKEN_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/token";
pub(crate) const INTERACTIVE_REDIRECT_URI: &str =
    "https://login.microsoftonline.com/common/oauth2/nativeclient";
pub(crate) const GRAPH_ME_URL: &str =
    "https://graph.microsoft.com/v1.0/me?$select=mail,userPrincipalName";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthSession {
    pub(crate) profile_id: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    pub(crate) verification_uri_complete: Option<String>,
    pub(crate) expires_in: u64,
    pub(crate) interval: u64,
    pub(crate) message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthPollResult {
    pub(crate) status: String,
    pub(crate) detail: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeviceCodeResponse {
    pub(crate) device_code: String,
    pub(crate) user_code: String,
    pub(crate) verification_uri: String,
    pub(crate) verification_uri_complete: Option<String>,
    pub(crate) expires_in: u64,
    pub(crate) interval: Option<u64>,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: Option<String>,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_in: Option<u64>,
    pub(crate) token_type: Option<String>,
    pub(crate) scope: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GraphMeResponse {
    pub(crate) mail: Option<String>,
    pub(crate) user_principal_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAuthSession {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_in: Option<u64>,
    pub(crate) token_type: Option<String>,
    pub(crate) scope: Option<String>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionSnapshot {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InteractiveAuthUpdatedEvent {
    pub(crate) profile_id: String,
    pub(crate) email: String,
    pub(crate) auth_configured: bool,
    pub(crate) agent_state: String,
}
