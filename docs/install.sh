#!/usr/bin/env bash
set -euo pipefail

# SomeDrive one-line installer for Linux.
#   curl -sf https://brumley.info/somedrive/install.sh | bash
# The in-app updater re-runs this same script, so keep its options stable.

REPO="jackbrumley/somedrive"
# The binary, the deb/rpm package, the desktop file and the icon all use this name.
BIN_NAME="somedrive"
INSTALL_BASE_URL="https://brumley.info/somedrive"

# GitHub API request headers. SOMEDRIVE_GITHUB_TOKEN is optional: anonymous
# requests are limited to 60 an hour per IP address.
GITHUB_API_HEADERS=(-H "Accept: application/vnd.github+json")
if [[ -n "${SOMEDRIVE_GITHUB_TOKEN:-}" ]]; then
  GITHUB_API_HEADERS+=(-H "Authorization: Bearer ${SOMEDRIVE_GITHUB_TOKEN}")
fi

usage() {
  cat <<EOF
SomeDrive installer

Usage:
  install.sh [--appimage] [--system] [--version <tag>] [--yes] [--no-relaunch] [--insecure-skip-verify]

Default: system-wide install via apt/dnf (asks for your password). Falls back to
the AppImage if no supported package manager or root access is available.

Options:
  --appimage              Install the AppImage to ~/.local/bin (no password needed)
  --system                Force a system-wide install via apt/dnf
  --version <tag>         Install a specific release tag (e.g. v0.1.0)
  --yes                   Skip the confirmation prompt
  --no-relaunch           Do not start SomeDrive after installing
  --insecure-skip-verify  Skip checksum verification (not recommended)
  -h, --help              Show this help message

Environment:
  SOMEDRIVE_GITHUB_TOKEN  GitHub token for API requests (avoids the anonymous rate limit)
  SOMEDRIVE_NO_RELAUNCH   Set to 1 to not start SomeDrive after installing

Examples:
  curl -sf ${INSTALL_BASE_URL}/install.sh | bash
  curl -sf ${INSTALL_BASE_URL}/install.sh | bash -s -- --appimage
EOF
}

LOG_DIR="${HOME}/.config/somedrive/debug"
mkdir -p "$LOG_DIR" 2>/dev/null || true
LOG_FILE="${LOG_DIR}/update.log"

log() {
  printf "[somedrive] %s\n" "$*"
  printf "[%s] [somedrive] %s\n" "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG_FILE" 2>/dev/null || true
}

warn() {
  printf "[somedrive] WARNING: %s\n" "$*" >&2
  printf "[%s] [somedrive] WARNING: %s\n" "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG_FILE" 2>/dev/null || true
}

fail() {
  printf "[somedrive] ERROR: %b\n" "$*" >&2
  printf "[%s] [somedrive] ERROR: %b\n" "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG_FILE" 2>/dev/null || true
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

optional_cmd() {
  command -v "$1" >/dev/null 2>&1
}

has_display() {
  [[ -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" ]]
}

run_as_root() {
  if [[ "$EUID" -eq 0 ]]; then
    "$@"
  elif optional_cmd sudo && sudo -n true 2>/dev/null; then
    sudo "$@"
  elif [[ -t 0 ]] && optional_cmd sudo; then
    sudo "$@"
  elif optional_cmd pkexec && has_display; then
    pkexec "$@"
  elif optional_cmd sudo; then
    sudo "$@"
  else
    fail "root permissions required but neither sudo nor pkexec is available"
  fi
}

# Returns 0 when root commands can run (already root, cached sudo, an
# interactive sudo prompt, or a graphical pkexec prompt).
acquire_root() {
  if [[ "$EUID" -eq 0 ]]; then
    return 0
  fi
  if optional_cmd sudo && sudo -n true 2>/dev/null; then
    return 0
  fi
  if [[ -t 0 ]]; then
    optional_cmd sudo || return 1
    sudo -v
  elif optional_cmd pkexec && has_display; then
    return 0
  elif optional_cmd sudo; then
    sudo -n true 2>/dev/null || return 1
  else
    return 1
  fi
}

stop_running_process() {
  if optional_cmd pgrep && pgrep -x "$BIN_NAME" >/dev/null 2>&1; then
    log "Closing running SomeDrive"
    if optional_cmd pkill; then
      pkill -TERM -x "$BIN_NAME" >/dev/null 2>&1 || true
      sleep 1
      pkill -KILL -x "$BIN_NAME" >/dev/null 2>&1 || true
    fi
  fi
}

resolve_arch() {
  case "$(uname -m)" in
    x86_64|amd64) printf "x64" ;;
    aarch64|arm64) fail "ARM64 builds of SomeDrive are not available yet; only x64 (x86_64) is supported." ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
  esac
}

detect_package_manager() {
  if optional_cmd dnf; then
    printf "dnf"
  elif optional_cmd apt-get; then
    printf "apt"
  else
    printf "none"
  fi
}

fetch_latest_release_tag() {
  local tag
  tag="$(curl -fsL --retry 3 --connect-timeout 10 "${GITHUB_API_HEADERS[@]}" \
    "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -o '"tag_name": *"[^"]*"' | head -n1 | sed 's/.*"\([^"]*\)"$/\1/')" || true
  [[ -n "$tag" ]] || fail "could not determine the latest release from GitHub (you may be rate limited; try again in a few minutes)"
  printf "%s" "$tag"
}

# GitHub computes a SHA-256 digest for every uploaded release asset and
# exposes it through the releases API; that is the source of truth.
verify_checksum() {
  local file="$1" asset_name="$2"
  local release_json expected actual
  release_json="$(curl -fsL --retry 3 --connect-timeout 10 "${GITHUB_API_HEADERS[@]}" \
    "https://api.github.com/repos/${REPO}/releases/tags/${release_tag}")" \
    || fail "could not fetch the checksum for ${asset_name} from GitHub. Try again later, or re-run with --insecure-skip-verify."
  if optional_cmd python3; then
    expected="$(printf "%s" "$release_json" | python3 -c '
import json, sys
name = sys.argv[1]
for asset in json.load(sys.stdin).get("assets", []):
    if asset.get("name") == name:
        print(asset.get("digest") or "")
' "$asset_name" 2>/dev/null)" || true
  else
    # No JSON parser: each asset lists "digest" shortly before its
    # "browser_download_url", with only flat fields in between.
    expected="$(printf "%s" "$release_json" | tr -d '\n' \
      | grep -o "\"digest\": *\"sha256:[0-9a-f]*\"[^{}]*\"browser_download_url\": *\"[^\"]*/${asset_name//./\\.}\"" \
      | grep -o 'sha256:[0-9a-f]*' | head -n1)" || true
  fi
  expected="${expected#sha256:}"
  [[ -n "$expected" ]] || fail "GitHub did not provide a checksum for ${asset_name}. Re-run with --insecure-skip-verify to bypass verification."
  actual="$(sha256sum "$file" | awk '{print $1}')"
  [[ "$expected" == "$actual" ]] || fail "checksum mismatch (expected $expected, got $actual)"
  log "Checksum verified"
}

remove_user_appimage() {
  local user_binary="${HOME}/.local/bin/${BIN_NAME}"
  local user_desktop="${HOME}/.local/share/applications/${BIN_NAME}.desktop"
  for path in "$user_binary" "$user_desktop"; do
    if [[ -e "$path" || -L "$path" ]]; then
      log "Removing previous user install: ${path}"
      rm -f "$path"
    fi
  done
}

install_system=false
install_appimage=false
non_interactive=false
skip_verify=false
relaunch=true
if [[ "${SOMEDRIVE_NO_RELAUNCH:-}" == "1" || "${SOMEDRIVE_NO_RELAUNCH:-}" == "true" ]]; then
  relaunch=false
fi
version=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --system) install_system=true ;;
    --appimage) install_appimage=true ;;
    --relaunch) relaunch=true ;;
    --no-relaunch) relaunch=false ;;
    --version)
      [[ $# -ge 2 ]] || fail "--version requires a value"
      version="$2"
      shift
      ;;
    --yes) non_interactive=true ;;
    --insecure-skip-verify) skip_verify=true ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
  shift
done

[[ "$(uname -s)" == "Linux" ]] || fail "SomeDrive is available for Linux only."

need_cmd curl
need_cmd sha256sum

# Piped into bash (curl | bash), stdin is the script, so prompts can't be answered.
if [[ ! -t 0 ]]; then
  non_interactive=true
fi

arch="$(resolve_arch)"
pm="$(detect_package_manager)"

if [[ "$install_appimage" == true ]]; then
  install_system=false
elif [[ "$install_system" == true ]]; then
  [[ "$pm" != "none" ]] || fail "no supported package manager found (dnf/apt). Use --appimage instead."
  log "System install requested; root access required"
  acquire_root || fail "could not get root access for a system install"
elif [[ "$pm" != "none" ]]; then
  log "Package manager '${pm}' detected; asking for root access"
  if acquire_root; then
    install_system=true
  elif [[ -x "/usr/bin/${BIN_NAME}" ]]; then
    fail "root access is required to update the existing system install in /usr/bin"
  else
    log "Root access unavailable; installing the AppImage instead"
  fi
else
  log "No supported package manager found; installing the AppImage"
fi

if [[ "$install_system" == true ]]; then
  if [[ "$pm" == "dnf" ]]; then package_ext="rpm"; else package_ext="deb"; fi
else
  package_ext="AppImage"
fi

if [[ -n "$version" ]]; then
  release_tag="$version"
else
  release_tag="$(fetch_latest_release_tag)"
fi

asset_name="${BIN_NAME}-${release_tag#v}-linux-${arch}.${package_ext}"
asset_url="https://github.com/${REPO}/releases/download/${release_tag}/${asset_name}"

log "Release: ${release_tag}"
if [[ "$install_system" == true ]]; then
  log "Install mode: system-wide (${pm}, .${package_ext})"
else
  log "Install mode: user (AppImage in ~/.local/bin)"
fi

if [[ "$non_interactive" == false ]]; then
  printf "Proceed with installation? [y/N] "
  read -r answer
  [[ "${answer:-}" == "y" || "${answer:-}" == "Y" ]] || fail "installation cancelled"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
package_path="${tmp_dir}/${asset_name}"

log "Downloading ${asset_url}"
curl -fL --retry 3 --connect-timeout 15 -o "$package_path" "$asset_url" \
  || fail "download failed: ${asset_name} was not found in release ${release_tag}."

if [[ "$skip_verify" == true ]]; then
  warn "checksum verification disabled"
else
  verify_checksum "$package_path" "$asset_name"
fi

stop_running_process

if [[ "$install_system" == true ]]; then
  # A user-local AppImage would shadow the system package, so remove it.
  remove_user_appimage
  log "Installing with ${pm}"
  if [[ "$pm" == "dnf" ]]; then
    run_as_root dnf install -y "$package_path"
  else
    run_as_root apt-get install -y "$package_path"
  fi
  log "System install complete"
else
  install_dir="${HOME}/.local/bin"
  desktop_dir="${HOME}/.local/share/applications"
  icon_dir="${HOME}/.local/share/icons/hicolor/256x256/apps"
  install -d "$install_dir" "$desktop_dir" "$icon_dir"

  appimage_path="${install_dir}/${BIN_NAME}"
  install -m 0755 "$package_path" "$appimage_path"
  log "AppImage installed at ${appimage_path}"

  # Pull the app icon out of the AppImage for the launcher.
  # (The top-level icon is a symlink, so take the real file from usr/share/icons;
  # Tauri puts its 256px icon in a folder named "256x256@2".)
  if (cd "$tmp_dir" && "$appimage_path" --appimage-extract 'usr/share/icons/*' >/dev/null 2>&1); then
    icon_src=""
    for size in "256x256@2" "256x256" "128x128"; do
      candidate="${tmp_dir}/squashfs-root/usr/share/icons/hicolor/${size}/apps/${BIN_NAME}.png"
      if [[ -f "$candidate" ]]; then icon_src="$candidate"; break; fi
    done
    if [[ -n "$icon_src" ]]; then
      cp "$icon_src" "${icon_dir}/${BIN_NAME}.png" 2>/dev/null || true
    fi
  fi

  cat > "${desktop_dir}/${BIN_NAME}.desktop" <<EOF
[Desktop Entry]
Name=SomeDrive
Comment=Linux-first desktop client for Microsoft OneDrive
Exec=${appimage_path}
Icon=${BIN_NAME}
Terminal=false
Type=Application
Categories=Network;FileTransfer;
StartupWMClass=${BIN_NAME}
StartupNotify=true
EOF
  log "Desktop launcher created"


  if [[ ":$PATH:" != *":${install_dir}:"* ]]; then
    log "Note: ${install_dir} is not in your PATH. Launch SomeDrive from your app menu."
  fi
fi

optional_cmd update-desktop-database && update-desktop-database "${HOME}/.local/share/applications" >/dev/null 2>&1 || true
optional_cmd gtk-update-icon-cache && gtk-update-icon-cache -f -t -q "${HOME}/.local/share/icons/hicolor" >/dev/null 2>&1 || true
if optional_cmd kbuildsycoca6; then
  kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
elif optional_cmd kbuildsycoca5; then
  kbuildsycoca5 --noincremental >/dev/null 2>&1 || true
fi

log "Installation complete. Launch SomeDrive from your application menu."

if [[ "$relaunch" == true ]] && has_display; then
  if [[ "$install_system" == true ]]; then
    launch_path="/usr/bin/${BIN_NAME}"
  else
    launch_path="${appimage_path}"
  fi
  if [[ -x "$launch_path" ]]; then
    log "Starting SomeDrive"
    nohup "$launch_path" >/dev/null 2>&1 &
  fi
fi
