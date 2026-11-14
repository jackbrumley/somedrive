<p align="center">
  <img src="src-tauri/icons/128x128.png" alt="" width="96" height="96" />
</p>

<h1 align="center">SomeDrive</h1>

<p align="center">A Linux-first desktop client for Microsoft OneDrive.<br />Sign in, pick a folder and sync, with clear status for every account and no command line needed.</p>

<p align="center">
  <a href="https://github.com/jackbrumley/somedrive/releases/latest"><img src="https://img.shields.io/github/v/release/jackbrumley/somedrive?label=release&color=1677d2" alt="Latest release" /></a>
  <img src="https://img.shields.io/badge/platform-Linux-1677d2" alt="Linux" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-1677d2" alt="AGPL-3.0" /></a>
</p>

<p align="center">
  <a href="https://somedrive.org">Website</a> ·
  <a href="https://github.com/jackbrumley/somedrive/releases/latest">Downloads</a> ·
  <a href="https://github.com/jackbrumley/somedrive/issues">Report an issue</a>
</p>

## Install

**Linux** (installs the `.deb` on Debian, Ubuntu and Mint, or the `.rpm` on Fedora, and asks for your password)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://somedrive.org/install.sh | bash
```

**Linux AppImage** (any distro, installs to `~/.local/bin`, no password needed)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://somedrive.org/install.sh | bash -s -- --appimage
```

**Uninstall**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://somedrive.org/uninstall.sh | bash
```

Prefer a package? The [latest release](https://github.com/jackbrumley/somedrive/releases/latest) has `.deb`, `.rpm` and `.AppImage` files. SomeDrive updates itself from **Settings → Check for Updates**.

## What it does

SomeDrive syncs Microsoft OneDrive folders to your Linux computer. Microsoft doesn't make a OneDrive app for Linux, so SomeDrive fills the gap with a simple desktop app: sign in, pick a folder, and see clearly what's synced for every account.

- Two-way OneDrive sync for personal and business accounts, side by side
- Sign in to Microsoft inside the app, no tokens or config files
- Pause and resume each account, with live activity and detailed logs
- Updates itself in place, and finishes in-flight transfers before it restarts

Each account syncs to `~/SomeDrive/<profile-slug>/` by default. SomeDrive is in active development, so its interface and stored data may still change. It is not affiliated with Microsoft.

## Build from source

```bash
npm install
npm run tauri:dev                 # run in development
npm run release:linux             # build release packages (needs podman)
node scripts/package-release.mjs  # collect them into release-artifacts/
```

Release packages land in `release-artifacts/`; [docs/RELEASE.md](docs/RELEASE.md) has the full procedure. Contributor notes are in [AGENTS.md](AGENTS.md).

## License

[GNU AGPLv3](LICENSE). Use it, learn from it and build on it freely; if you improve it, share the changes back.

---

<p align="center">
  SomeDrive is one of the free apps by <a href="https://brumley.info">Jack Brumley</a>:<br />
  <a href="https://voquill.org">Voquill</a> · <a href="https://brumley.info/rusty-g6/">Rusty G6</a> · <b>SomeDrive</b> · <a href="https://yambuck.com/">Yambuck</a> · <a href="https://brumley.info/rawtap/">RawTap</a> · <a href="https://bmuster.com/">BMuster</a>
</p>
