fn clear_pending_interactive_auth(app: &tauri::AppHandle, profile_id: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut lock = state
        .interactive_auth_pending
        .lock()
        .map_err(|_| "Interactive auth lock is poisoned".to_string())?;
    lock.remove(profile_id);
    Ok(())
}

fn domain_hint_for_kind(kind: &str) -> Result<&'static str, String> {
    match kind.trim().to_lowercase().as_str() {
        "personal" => Ok("consumers"),
        "business" => Ok("organizations"),
        _ => Err("Unsupported account kind for interactive auth".to_string()),
    }
}

fn resolve_interactive_client_id() -> String {
    std::env::var("ONEDRIVE_INTERACTIVE_CLIENT_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_INTERACTIVE_CLIENT_ID.to_string())
}

fn sanitize_auth_window_label(profile_id: &str) -> String {
    let mut label = String::from("auth-");
    for character in profile_id.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            label.push(character);
        }
    }
    if label == "auth-" {
        label.push_str("profile");
    }
    label
}

fn generate_state_token() -> String {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let pid = std::process::id();
    format!("s{}{}", now, pid)
}

fn url_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        let should_encode = !matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if should_encode {
            encoded.push('%');
            encoded.push_str(&format!("{:02X}", byte));
        } else {
            encoded.push(byte as char);
        }
    }
    encoded
}

async fn fetch_account_email(access_token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(GRAPH_ME_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| format!("Failed requesting Microsoft profile: {error}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed reading Microsoft profile response body: {error}"))?;

    if !status.is_success() {
        let snippet: String = body.chars().take(400).collect();
        return Err(format!(
            "Microsoft profile request failed with status {status}. Response: {snippet}"
        ));
    }

    let payload: GraphMeResponse = serde_json::from_str(&body).map_err(|error| {
        let snippet: String = body.chars().take(400).collect();
        format!("Failed to decode Microsoft profile response: {error}. Response snippet: {snippet}")
    })?;

    let email = payload
        .mail
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .user_principal_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_string());

    Ok(email)
}

fn auth_session_file_path(profile_id: &str) -> Result<PathBuf, String> {
    Ok(crate::paths::account_dir(profile_id)?.join("auth.json"))
}

fn write_auth_session(profile_id: &str, session: StoredAuthSession) -> Result<(), String> {
    // Atomic and owner-only: this file holds the account's refresh token.
    crate::app::json_store::write_json(&auth_session_file_path(profile_id)?, &session)
}

pub fn load_auth_session(profile_id: &str) -> Result<AuthSessionSnapshot, String> {
    // Missing, or unparseable and moved aside: the account must sign in again.
    let session: StoredAuthSession =
        crate::app::json_store::read_json(&auth_session_file_path(profile_id)?)?
            .ok_or_else(|| "Account auth session not found".to_string())?;
    Ok(AuthSessionSnapshot {
        access_token: session.access_token,
        refresh_token: session.refresh_token,
        expires_in: session.expires_in,
        token_type: session.token_type,
        scope: session.scope,
        updated_at: session.updated_at,
    })
}

/// Why a token refresh failed. `SignInRequired` means Microsoft will not
/// accept the saved sign-in again (expired, revoked or needing consent):
/// retrying cannot help and the account must sign in again.
#[derive(Debug)]
pub enum TokenRefreshError {
    SignInRequired(String),
    Failed(String),
}

impl std::fmt::Display for TokenRefreshError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignInRequired(detail) => write!(formatter, "Sign-in required: {detail}"),
            Self::Failed(detail) => write!(formatter, "{detail}"),
        }
    }
}

/// OAuth error codes that mean the refresh token can never succeed again.
fn refresh_error_requires_sign_in(error_code: &str) -> bool {
    matches!(
        error_code,
        "invalid_grant" | "interaction_required" | "consent_required" | "login_required"
    )
}

pub async fn refresh_access_token(
    profile_id: &str,
) -> Result<AuthSessionSnapshot, TokenRefreshError> {
    let existing = load_auth_session(profile_id).map_err(TokenRefreshError::SignInRequired)?;
    let refresh_token = existing.refresh_token.clone().ok_or_else(|| {
        TokenRefreshError::SignInRequired("No saved refresh token for this account".to_string())
    })?;

    let client_id = resolve_interactive_client_id();
    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("scope", DEFAULT_SCOPE),
        ])
        .send()
        .await
        .map_err(|error| {
            TokenRefreshError::Failed(format!("Failed refreshing access token: {error}"))
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| {
            TokenRefreshError::Failed(format!(
                "Failed reading refresh-token response body: {error}"
            ))
        })?;

    let payload: TokenResponse = serde_json::from_str(&body).map_err(|error| {
        let snippet: String = body.chars().take(400).collect();
        TokenRefreshError::Failed(format!(
            "Failed to decode refresh-token response: {error}. Response snippet: {snippet}"
        ))
    })?;

    if !status.is_success() || payload.error.is_some() {
        let detail = payload
            .error_description
            .unwrap_or_else(|| "Unknown token refresh error".to_string());
        let code = payload.error.unwrap_or_else(|| status.to_string());
        let message = format!("Token refresh failed: {code} ({detail})");
        if refresh_error_requires_sign_in(&code) {
            return Err(TokenRefreshError::SignInRequired(message));
        }
        return Err(TokenRefreshError::Failed(message));
    }

    let access_token = payload
        .access_token
        .ok_or_else(|| {
            TokenRefreshError::Failed("Refresh response missing access token".to_string())
        })?;
    let next_refresh_token = payload.refresh_token.or(existing.refresh_token);
    let updated_at = chrono::Local::now().to_rfc3339();

    write_auth_session(
        profile_id,
        StoredAuthSession {
            access_token: access_token.clone(),
            refresh_token: next_refresh_token.clone(),
            expires_in: payload.expires_in,
            token_type: payload.token_type.clone(),
            scope: payload.scope.clone(),
            updated_at: updated_at.clone(),
        },
    )
    .map_err(TokenRefreshError::Failed)?;

    log::info!(
        "{} Access token refreshed",
        log_context::account_prefix(profile_id)
    );

    Ok(AuthSessionSnapshot {
        access_token,
        refresh_token: next_refresh_token,
        expires_in: payload.expires_in,
        token_type: payload.token_type,
        scope: payload.scope,
        updated_at,
    })
}

/// Records that the account's saved sign-in is no longer accepted, so it
/// stays signed out (and its sync stopped) across restarts until the user
/// signs in again. The account keeps its email so the UI can name it.
pub fn mark_account_sign_in_required(
    profiles_lock: &std::sync::Mutex<()>,
    profile_id: &str,
) -> Result<(), String> {
    let _guard = profiles_lock
        .lock()
        .map_err(|_| "Account profile lock is poisoned".to_string())?;
    let mut profiles = load_profiles()?;
    let profile = profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| "Account profile not found".to_string())?;
    if !profile.auth_configured {
        return Ok(());
    }
    profile.auth_configured = false;
    save_profiles(&profiles)?;
    log::warn!(
        "{} AUTH_SIGN_IN_REQUIRED profile marked signed out",
        log_context::account_prefix(profile_id)
    );
    Ok(())
}

#[cfg(test)]
mod token_refresh_error_tests {
    use super::refresh_error_requires_sign_in;

    #[test]
    fn expired_or_revoked_grants_require_sign_in() {
        for code in [
            "invalid_grant",
            "interaction_required",
            "consent_required",
            "login_required",
        ] {
            assert!(refresh_error_requires_sign_in(code), "{code}");
        }
    }

    #[test]
    fn transient_token_errors_do_not_require_sign_in() {
        for code in ["temporarily_unavailable", "server_error", "503 Service Unavailable"] {
            assert!(!refresh_error_requires_sign_in(code), "{code}");
        }
    }
}
