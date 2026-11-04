use super::*;

#[tauri::command]
pub async fn start_device_auth(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<DeviceAuthSession, String> {
    let account_prefix = log_context::account_prefix(&profile_id);
    log::info!("{} Starting device auth", account_prefix);
    {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let profiles = load_profiles()?;
        if !profiles.iter().any(|profile| profile.id == profile_id) {
            return Err("Account profile not found".to_string());
        }
    }

    let client = reqwest::Client::new();
    let response = client
        .post(DEVICE_CODE_URL)
        .form(&[
            ("client_id", DEFAULT_MS_CLIENT_ID),
            ("scope", DEFAULT_SCOPE),
        ])
        .send()
        .await
        .map_err(|error| format!("Failed to request device code: {error}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed reading device code response body: {error}"))?;

    if !status.is_success() {
        let snippet: String = body.chars().take(400).collect();
        return Err(format!(
            "Device code request failed with status {status}. Response: {snippet}"
        ));
    }

    let payload: DeviceCodeResponse = serde_json::from_str(&body).map_err(|error| {
        let snippet: String = body.chars().take(400).collect();
        format!("Failed to decode device code response: {error}. Response snippet: {snippet}")
    })?;

    log::info!(
        "{} Device auth session created verification_uri={} expires_in={}s",
        account_prefix,
        payload.verification_uri,
        payload.expires_in
    );

    {
        let mut pending = state
            .auth_pending
            .lock()
            .map_err(|_| "Auth pending lock is poisoned".to_string())?;
        pending.insert(
            profile_id.clone(),
            DeviceAuthPending {
                profile_id: profile_id.clone(),
                device_code: payload.device_code.clone(),
                interval_secs: payload.interval.unwrap_or(5),
            },
        );
    }

    Ok(DeviceAuthSession {
        profile_id,
        user_code: payload.user_code,
        verification_uri: payload.verification_uri,
        verification_uri_complete: payload.verification_uri_complete,
        expires_in: payload.expires_in,
        interval: payload.interval.unwrap_or(5),
        message: payload.message,
    })
}

#[tauri::command]
pub async fn poll_device_auth(
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<DeviceAuthPollResult, String> {
    let account_prefix = log_context::account_prefix(&profile_id);
    log::info!("{} Polling device auth token", account_prefix);
    let pending = {
        let lock = state
            .auth_pending
            .lock()
            .map_err(|_| "Auth pending lock is poisoned".to_string())?;
        lock.get(&profile_id)
            .map(|entry| {
                (
                    entry.profile_id.clone(),
                    entry.device_code.clone(),
                    entry.interval_secs,
                )
            })
            .ok_or_else(|| "No active auth session for this profile".to_string())?
    };

    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", DEFAULT_MS_CLIENT_ID),
            ("device_code", pending.1.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("Failed to poll token endpoint: {error}"))?;

    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed reading token response body: {error}"))?;

    let payload: TokenResponse = serde_json::from_str(&body).map_err(|error| {
        let snippet: String = body.chars().take(400).collect();
        format!("Failed to decode token response: {error}. Response snippet: {snippet}")
    })?;

    if let Some(error) = payload.error.clone() {
        if error == "authorization_pending" || error == "slow_down" {
            log::info!("{} Device auth pending status={}", account_prefix, error);
            return Ok(DeviceAuthPollResult {
                status: "pending".to_string(),
                detail: payload
                    .error_description
                    .unwrap_or_else(|| "Waiting for user approval".to_string()),
            });
        }

        {
            let mut lock = state
                .auth_pending
                .lock()
                .map_err(|_| "Auth pending lock is poisoned".to_string())?;
            lock.remove(&profile_id);
        }

        return Ok(DeviceAuthPollResult {
            status: "error".to_string(),
            detail: payload
                .error_description
                .unwrap_or_else(|| format!("Authentication failed: {error}")),
        });
    }

    let access_token = payload
        .access_token
        .ok_or_else(|| "Token response missing access token".to_string())?;
    let account_email = fetch_account_email(&access_token).await?;
    let updated_at = chrono::Local::now().to_rfc3339();

    write_auth_session(
        &profile_id,
        StoredAuthSession {
            access_token,
            refresh_token: payload.refresh_token,
            expires_in: payload.expires_in,
            token_type: payload.token_type,
            scope: payload.scope,
            updated_at,
        },
    )?;

    {
        let _guard = state
            .profiles_lock
            .lock()
            .map_err(|_| "Account profile lock is poisoned".to_string())?;
        let mut profiles = load_profiles()?;
        if let Some(profile) = profiles.iter_mut().find(|profile| profile.id == profile_id) {
            profile.auth_configured = true;
            profile.email = account_email.clone();
        }
        save_profiles(&profiles)?;
    }

    {
        let mut lock = state
            .auth_pending
            .lock()
            .map_err(|_| "Auth pending lock is poisoned".to_string())?;
        lock.remove(&profile_id);
    }

    sync_engine::runtime_set_profile_auth_ready(&state.sync_runtime, &profile_id, true);

    let _ = activity_log::append_event(
        &profile_id,
        &account_email,
        "success",
        &format!(
            "{} Authentication completed successfully",
            log_context::account_prefix_from_parts(&profile_id, &account_email)
        ),
    );
    log::info!(
        "{} Device auth completed",
        log_context::account_prefix_from_parts(&profile_id, &account_email)
    );

    Ok(DeviceAuthPollResult {
        status: "authorized".to_string(),
        detail: "Authentication complete".to_string(),
    })
}
