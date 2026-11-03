// Single source of truth for release identity: the version, the git tag, the
// release asset file names, and their download URLs. package-release.mjs names
// the build outputs from here and release-notes-template.mjs writes the
// Downloads block from here, so the two can never disagree.
//
// install.sh and the in-app updater derive download URLs from the
// tag and these exact file names, so a wrong name breaks updates and installs.
// See RELEASE_STANDARD.md (jackbrumley/jackbrumley.github.io) and docs/RELEASE.md.

import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_CONF = JSON.parse(readFileSync(resolve(ROOT, "src-tauri", "tauri.conf.json"), "utf8"));

export const REPOSITORY = "jackbrumley/somedrive";

export const VERSION = TAURI_CONF.version;
export const TAG = `v${VERSION}`;

export const ASSET_NAMES = {
  linuxDeb: `somedrive-${VERSION}-linux-x64.deb`,
  linuxRpm: `somedrive-${VERSION}-linux-x64.rpm`,
  linuxAppImage: `somedrive-${VERSION}-linux-x64.AppImage`,
};

// Every asset a complete release must carry. verify-release.mjs checks the
// published release against this list before it may be promoted.
export const EXPECTED_ASSETS = Object.values(ASSET_NAMES);

// Order and labels of the Downloads block in every release's notes.
export const DOWNLOAD_LINKS = [
  { platform: "Linux (Debian/Ubuntu)", label: ".deb", fileName: ASSET_NAMES.linuxDeb },
  { platform: "Linux (Fedora/RHEL)", label: ".rpm", fileName: ASSET_NAMES.linuxRpm },
  { platform: "Linux (Portable)", label: ".AppImage", fileName: ASSET_NAMES.linuxAppImage },
];

// The tag (with its `v`) is the URL path segment; the file name carries the
// bare version.
export function downloadUrl(fileName) {
  return `https://github.com/${REPOSITORY}/releases/download/${TAG}/${fileName}`;
}

// Asset names are `somedrive-<version>-<os>-<arch>.<ext>`. A plain
// MAJOR.MINOR.PATCH version keeps them unambiguous (a suffix like `-beta.1`
// would add hyphens that collide with the separators).
export function assertPlainVersion() {
  if (!/^\d+\.\d+\.\d+$/.test(VERSION)) {
    console.error(`Release version '${VERSION}' must be plain MAJOR.MINOR.PATCH (no pre-release or build suffix).`);
    process.exit(1);
  }
}
