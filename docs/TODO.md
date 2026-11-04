# TODO

Plans and known bugs, per the App Standard (section 8).

## Now

### Sync engine: baseline reconciliation
The current engine is not safe to run two-way on real data (data-safety
review, 2026-11-04). Design: `docs/sync-engine.md`. Built on branch
`sync-baseline`; worked through in this order:

1. [x] Test isolation (`cargo test` must not write to `~/.config/somedrive`).
2. [x] Baseline engine: cloud, local and baseline state per path, delta
   pages committed atomically, decision-table planner, safe downloads and
   uploads, deletes to Trash through the two-way guard, missing-root stop,
   two-phase lifecycle removed.
3. [x] Hold conflicts instead of copying them; a Sync problems tab where the
   user keeps this device's version, OneDrive's version, or both (one file
   or many at once).
4. [x] Robustness: an expired delta token (410 / `resyncRequired`) starts a
   fresh enumeration; sign-in expiring mid-download requeues the download
   instead of failing it; a failed cycle retries automatically with backoff.
5. [x] True local rename for cloud folder renames (today a cloud rename
   re-downloads the folder and the delete guard asks before removing the
   old copy).
6. [ ] Test-account trial run (`docs/sync-engine.md` section 11) before any
   real data; then merge `sync-baseline` into `dev`.
7. [x] Optional setting "When a file changes on both sides: Ask me / Keep
   the newest automatically" (default Ask me).
8. [x] Exclude rules (Settings): folders and patterns never synced, such as
   browser caches.
9. [x] Typed `SyncError` through the engine instead of classifying errors by
   their text (`classify_sync_issue`).
10. [x] Replace the `include!` modules in `app/sync_engine.rs` and
    `app/auth.rs` with real Rust modules.
11. [x] Split the files over the 600-line soft budget
    (`npm run check:structure` lists them).

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
