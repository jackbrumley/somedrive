# Release Process

SomeDrive follows the
[Release and Update Standard](https://github.com/jackbrumley/jackbrumley.github.io/blob/main/RELEASE_STANDARD.md),
the same process as Voquill, Rusty G6 and Yambuck. This is SomeDrive's copy of
the procedure; the standard has the reasoning and the full installer and
updater contracts.

Release identity (tag, title, asset names, download links) is consumed by
`docs/install.sh` and the in-app updater, so a format
mistake breaks updates and one-line installs for every user.

## Format

| Where | Format | Example |
|---|---|---|
| `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` | `X.Y.Z` | `0.2.0` |
| Version bump commit | `chore: bump version to X.Y.Z` | |
| Git tag and release title | `vX.Y.Z` | `v0.2.0` |
| First line of the release notes | `# SomeDrive vX.Y.Z` | |
| Assets | `somedrive-X.Y.Z-linux-x64.<ext>` | `somedrive-0.2.0-linux-x64.deb` |

SomeDrive ships for Linux only. A complete release carries three assets, all
named by `scripts/release-assets.mjs`:
- `somedrive-X.Y.Z-linux-x64.deb`
- `somedrive-X.Y.Z-linux-x64.rpm`
- `somedrive-X.Y.Z-linux-x64.AppImage`

## Branches

Development happens on `dev`; releases are tagged on `dev`. Promotion
fast-forwards `main` to the release tag, so `main` always equals the latest
release. GitHub Pages serves `somedrive.org` (install page and
installer scripts) from `main:/docs`, so promotion also deploys installer
changes. Never commit to `main` directly.

## 1. Prepare and Tag

```bash
npm run release:prepare -- X.Y.Z
git push origin dev vX.Y.Z
```

Pushing the tag starts the **Release** workflow
(`.github/workflows/release.yml`):
1. **Prepare** checks the tag matches `tauri.conf.json` and creates the
   pre-release with the notes skeleton.
2. **Build** runs the Linux build inside the Ubuntu 22.04 container
   (`npm run release:linux`) and uploads the assets.
3. **Verify** fails unless the release carries exactly the three assets, each
   with a SHA-256 digest.
4. **Install test** installs the pre-release with the installer scripts from
   the tag: `.deb`, AppImage and uninstall.

Follow it with:
```bash
gh run watch "$(gh run list --workflow release.yml --limit 1 --json databaseId --jq '.[0].databaseId')" --exit-status
```

## 2. Write the Release Notes

```bash
npm run --silent release:notes > release-notes.md   # commits since the last tag print to the terminal
gh release edit vX.Y.Z --notes-file release-notes.md
```

Replace every `TODO`. Structure: `# SomeDrive vX.Y.Z`, one summary paragraph,
themed `###` sections of `- Label: sentence` bullets separated by `---`, then
the generated Downloads block (never edited). Write for users, not developers.

## 3. Promote

```bash
npm run release:promote
```

Refuses unless all three assets have digests and the notes contain no `TODO`.
It marks the release latest and fast-forwards `main` to the tag. From then on
the in-app updater and the one-line installers serve the new version.

## 4. Verify

- An older installed version offers the update and **Update Now** finishes
  in-flight transfers, installs and relaunches it, and syncing accounts resume.
- `curl --proto '=https' --tlsv1.2 -sSf https://somedrive.org/install.sh | bash` installs the new
  version.

## When Something Fails

- **Transient failure** (network, runner): `gh run rerun <run-id> --failed`.
- **Code failure:** nothing was promoted, so nobody received it. Fix on `dev`,
  then:
  ```bash
  gh release delete vX.Y.Z --cleanup-tag --yes
  git tag -d vX.Y.Z
  git tag vX.Y.Z
  git push origin dev vX.Y.Z
  ```
- **Dry run** of every build without a release:
  `gh workflow run release.yml --ref dev`.

## Building Locally (Fallback)

- Linux (needs `podman`): `npm run release:linux`, then
  `node scripts/package-release.mjs`.
- Upload with `gh release upload vX.Y.Z release-artifacts/*`.

## Agent Checklist

- [ ] Version prepared with `npm run release:prepare`; tag is `vX.Y.Z`.
- [ ] Release workflow green: build, verify and install test.
- [ ] Notes from `npm run release:notes`; no `TODO`; Downloads block untouched.
- [ ] Assets only from the workflow or `package-release.mjs`.
- [ ] `npm run release:promote` only when the maintainer asks.
- [ ] No AI attribution in commits, tags or notes.
