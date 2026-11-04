# TODO

Plans and known bugs, per the App Standard (section 8).

## Now

### Sync engine: baseline reconciliation
The current engine is not safe to run two-way on real data (data-safety
review, 2026-11-04). Design and build order: `docs/sync-engine.md`.
- [ ] Test isolation (`cargo test` must not write to `~/.config/somedrive`).
- [ ] `sync_items` with cloud, local and baseline state; delta pages
  committed atomically; JSON caches removed.
- [ ] Planner from the decision table; jobs re-queued from any state.
- [ ] Safe execution: safe overwrite, mtime, fsync and hash check,
  `If-Match` uploads, conflict copies, baseline commit.
- [ ] Deletes: Trash, two-direction guard re-checked on approval, missing
  root stop.
- [ ] Remove the two-phase lifecycle.
- [ ] Robustness: 410 resync, auth expiry requeue, cycle retry, 404 re-check,
  name and case issues, temp cleanup.
- [ ] Test-account scenario run (`docs/sync-engine.md` section 11).

### Project-owned Microsoft Entra app registration
SomeDrive must not ship with a third-party client ID.
- [ ] Create a Microsoft Entra app registration owned by this project that
  supports both personal Microsoft accounts (MSA) and work/school accounts
  (Entra ID).
- [ ] Branding for trustworthy consent screens: final app name,
  publisher/support URL, privacy policy URL, terms of service URL (if any),
  logo.
- [ ] Redirect URI for desktop interactive auth:
  `https://login.microsoftonline.com/common/oauth2/nativeclient`; enable the
  public client / native flow and the authorization code flow.
- [ ] Microsoft Graph delegated permissions: `offline_access`, `User.Read`
  and the OneDrive/Files scopes needed for sync, reduced to least privilege;
  admin consent where organizations require it.
- [ ] Test with `ONEDRIVE_INTERACTIVE_CLIENT_ID=<new-client-id>`, confirm no
  third-party consent identity is shown, then make the new ID the default.
  Keep the env var override for emergencies and staging.

## Next

### Auth QA matrix
- [ ] Personal account: add account, in-app sign-in, login + consent, token
  exchange succeeds, account shows authenticated.
- [ ] Business account: the same, tenant policies permitting.
- [ ] Redirect safety: callback state verification passes; mismatch and error
  paths are logged clearly.
- [ ] Logs carry the auth routing details for each attempt (account kind,
  `domain_hint`, `client_id`, authorize/token/redirect URLs).

### Documentation
- [ ] README section on auth setup ownership and the app registration.
- [ ] An auth setup doc with the redirect URI, permissions and the
  `ONEDRIVE_INTERACTIVE_CLIENT_ID` override.

### Release hardening
- [ ] Consent text and branding match the final product name everywhere.
- [ ] Final regression of add-account, auth completion, log copy/open and the
  account settings UX.

### Sync engine parameter bundling
- [ ] Bundle the structured activity contract (stage, progress_mode, current,
  total, unit, detail, cycle_id) into one struct passed through
  `runtime_set_current_activity`, `sync_runtime::set_current_activity` and
  the lifecycle store, and give the remote pipeline loop, local-change
  application and download retry a cycle context struct. Removes the
  commented `#[allow(clippy::too_many_arguments)]` markers.

## Ideas

- Show only the relevant tray item (Pause all or Resume all syncing) by
  rebuilding the tray menu when sync state changes.
- Split the remaining files over the 1000-line budget (see the allowlist in
  `scripts/check-file-lines.mjs`).
