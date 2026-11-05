use super::*;

#[tauri::command]
pub async fn start_interactive_auth(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    profile_id: String,
) -> Result<(), String> {
    let account_prefix = log_context::account_prefix(&profile_id);
    let interactive_client_id = resolve_interactive_client_id();
    let account_kind = {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let profiles = load_profiles()?;
        profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.kind.clone())
            .ok_or_else(|| "Account profile not found".to_string())?
    };

    let domain_hint = domain_hint_for_kind(&account_kind)?;
    let authorize_url = INTERACTIVE_AUTHORIZE_URL.to_string();
    let token_url = INTERACTIVE_TOKEN_URL.to_string();
    let redirect_uri = INTERACTIVE_REDIRECT_URI.to_string();

    {
        log::info!(
            "{} Interactive auth routing kind={} domain_hint={} client_id={} authorize_url={} token_url={} redirect_uri={}",
            account_prefix,
            account_kind,
            domain_hint,
            interactive_client_id,
            authorize_url,
            token_url,
            redirect_uri
        );
    }

    let window_label = sanitize_auth_window_label(&profile_id);
    let state_token = generate_state_token();
    let authorize_request_url = format!(
        "{}?client_id={}&scope={}&response_type=code&prompt=login&redirect_uri={}&state={}&domain_hint={}",
        authorize_url,
        interactive_client_id,
        url_encode(DEFAULT_SCOPE),
        url_encode(&redirect_uri),
        url_encode(&state_token),
        url_encode(domain_hint)
    );

    {
        let mut pending = state
            .interactive_auth_pending
            .lock()
            .map_err(|_| "Interactive auth lock is poisoned".to_string())?;
        pending.insert(
            profile_id.clone(),
            InteractiveAuthPending {
                profile_id: profile_id.clone(),
                state: state_token.clone(),
                window_label: window_label.clone(),
                client_id: interactive_client_id.to_string(),
                redirect_uri: redirect_uri.clone(),
                token_url: token_url.clone(),
            },
        );
    }

    if let Some(existing_window) = app.get_webview_window(&window_label) {
        let _ = existing_window.close();
    }

    log::info!("{} Opening interactive auth window", account_prefix);

    let callback_profile_id = profile_id.clone();
    let callback_window_label = window_label.clone();
    let callback_app = app.clone();

    WebviewWindowBuilder::new(
        &app,
        &window_label,
        WebviewUrl::External(
            authorize_request_url
                .parse()
                .map_err(|error| format!("Failed to parse auth URL: {error}"))?,
        ),
    )
    .title("Microsoft Sign-In")
    .inner_size(980.0, 760.0)
    .resizable(true)
    .center()
    .on_navigation(move |url| {
        let as_text = url.as_str();
        if !as_text.starts_with(&redirect_uri) {
            return true;
        }

        let app_handle = callback_app.clone();
        let profile_id = callback_profile_id.clone();
        let window_label = callback_window_label.clone();
        let callback = as_text.to_string();

        tauri::async_runtime::spawn(async move {
            match complete_interactive_auth_from_callback(&app_handle, &profile_id, &callback).await
            {
                Ok(()) => {
                    log::info!(
                        "{} Interactive auth completed",
                        log_context::account_prefix(&profile_id)
                    );
                }
                Err(error) => {
                    log::error!(
                        "{} Interactive auth failed: {}",
                        log_context::account_prefix(&profile_id),
                        error
                    );
                    let _ = activity_log::append_event(
                        &profile_id,
                        &profile_id,
                        "error",
                        &format!(
                            "{} Interactive authentication failed: {error}",
                            log_context::account_prefix(&profile_id)
                        ),
                    );
                }
            }

            if let Some(window) = app_handle.get_webview_window(&window_label) {
                let _ = window.close();
            }
        });

        false
    })
    .build()
    .map_err(|error| format!("Failed to open interactive sign-in window: {error}"))?;

    Ok(())
}

#[tauri::command]
pub fn clear_account_auth(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let mut profiles = load_profiles()?;
        let profile = profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "Account profile not found".to_string())?;
        profile.auth_configured = false;
        profile.email = String::new();
        save_profiles(&profiles)?;
    }

    let auth_path = auth_session_file_path(&profile_id)?;
    if auth_path.exists() {
        fs::remove_file(auth_path).map_err(|error| error.to_string())?;
    }

    sync_engine::runtime_set_profile_auth_ready(&state.sync_runtime, &profile_id, false);

    Ok(())
}

pub(crate) async fn complete_interactive_auth_from_callback(
    app: &tauri::AppHandle,
    profile_id: &str,
    callback_url: &str,
) -> Result<(), String> {
    let callback = callback_url
        .parse::<tauri::Url>()
        .map_err(|error| format!("Invalid callback URL: {error}"))?;

    let mut code: Option<String> = None;
    let mut returned_state: Option<String> = None;
    let mut oauth_error: Option<String> = None;
    let mut oauth_error_description: Option<String> = None;

    for (key, value) in callback.query_pairs() {
        let key_text = key.as_ref();
        if key_text == "code" {
            code = Some(value.to_string());
        } else if key_text == "state" {
            returned_state = Some(value.to_string());
        } else if key_text == "error" {
            oauth_error = Some(value.to_string());
        } else if key_text == "error_description" {
            oauth_error_description = Some(value.to_string());
        }
    }

    let app_state = app.state::<AppState>();

    let pending = {
        let lock = app_state
            .interactive_auth_pending
            .lock()
            .map_err(|_| "Interactive auth lock is poisoned".to_string())?;
        lock.get(profile_id)
            .map(|entry| {
                (
                    entry.profile_id.clone(),
                    entry.state.clone(),
                    entry.window_label.clone(),
                    entry.client_id.clone(),
                    entry.redirect_uri.clone(),
                    entry.token_url.clone(),
                )
            })
            .ok_or_else(|| "No pending interactive auth session for this profile".to_string())?
    };

    if !callback_url.starts_with(&pending.4) {
        clear_pending_interactive_auth(app, profile_id)?;
        return Err(format!(
            "OAuth callback redirect mismatch. Expected prefix '{}'",
            pending.4
        ));
    }

    if let Some(error_code) = oauth_error {
        let detail = oauth_error_description.unwrap_or_else(|| "Unknown OAuth error".to_string());
        clear_pending_interactive_auth(app, profile_id)?;
        return Err(format!("OAuth sign-in failed: {error_code} ({detail})"));
    }

    let state_value = returned_state.ok_or_else(|| "OAuth callback missing state".to_string())?;
    if state_value != pending.1 {
        clear_pending_interactive_auth(app, profile_id)?;
        return Err("OAuth callback state mismatch".to_string());
    }

    let authorization_code =
        code.ok_or_else(|| "OAuth callback missing authorization code".to_string())?;

    let client = reqwest::Client::new();
    let response = client
        .post(&pending.5)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", pending.3.as_str()),
            ("code", authorization_code.as_str()),
            ("redirect_uri", pending.4.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("Failed to exchange authorization code: {error}"))?;

    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed reading authorization token response body: {error}"))?;

    let payload: TokenResponse = serde_json::from_str(&body).map_err(|error| {
        let snippet: String = body.chars().take(400).collect();
        format!(
            "Failed to decode authorization token response: {error}. Response snippet: {snippet}"
        )
    })?;

    if let Some(error) = payload.error.clone() {
        clear_pending_interactive_auth(app, profile_id)?;
        return Err(payload
            .error_description
            .unwrap_or_else(|| format!("Authentication failed: {error}")));
    }

    let access_token = payload
        .access_token
        .ok_or_else(|| "Token response missing access token".to_string())?;
    let account_email = fetch_account_email(&access_token).await?;
    let updated_at = chrono::Local::now().to_rfc3339();
    log::info!(
        "{} AUTH_INTERACTIVE_TOKEN_EXCHANGE_SUCCESS email={}",
        log_context::account_prefix(profile_id),
        account_email
    );

    write_auth_session(
        profile_id,
        StoredAuthSession {
            access_token,
            refresh_token: payload.refresh_token,
            expires_in: payload.expires_in,
            token_type: payload.token_type,
            scope: payload.scope,
            updated_at,
        },
    )?;

    let updated_profile = {
        let _guard = app_state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let mut profiles = load_profiles()?;
        let profile = profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "Account profile not found during auth callback update".to_string())?;
        profile.auth_configured = true;
        profile.email = account_email.clone();
        profile.agent_state = "syncing".to_string();
        let updated = profile.clone();
        save_profiles(&profiles)?;
        updated
    };

    log::info!(
        "{} AUTH_INTERACTIVE_PROFILE_UPDATED profile_id={} email={} auth_configured={} agent_state={}",
        log_context::account_prefix_from_parts(profile_id, &updated_profile.email),
        updated_profile.id,
        updated_profile.email,
        updated_profile.auth_configured,
        updated_profile.agent_state
    );

    log::info!(
        "{} AUTH_INTERACTIVE_AUTO_START_SYNC requested",
        log_context::account_prefix_from_parts(profile_id, &updated_profile.email)
    );
    sync_engine::runtime_set_profile_auth_ready(&app_state.sync_runtime, profile_id, true);
    sync_engine::on_agent_state_changed(&app_state, profile_id, "syncing")?;
    log::info!(
        "{} AUTH_INTERACTIVE_AUTO_START_SYNC_RESULT success",
        log_context::account_prefix_from_parts(profile_id, &updated_profile.email)
    );

    // The window hears about the new sign-in from the profile and auth.json
    // writes above (docs/ui-data-flow.md).
    clear_pending_interactive_auth(app, profile_id)?;

    let _ = activity_log::append_event(
        profile_id,
        &account_email,
        "success",
        &format!(
            "{} Interactive authentication completed successfully",
            log_context::account_prefix_from_parts(profile_id, &account_email)
        ),
    );

    Ok(())
}
