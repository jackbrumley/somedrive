# TODO

Plans and known bugs, per the App Standard (section 8).

## Now

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

## Ideas

- Show only the relevant tray item (Pause all or Resume all syncing) by
  rebuilding the tray menu when sync state changes.
- Split the remaining files over the 1000-line budget (see the allowlist in
  `scripts/check-file-lines.mjs`).
