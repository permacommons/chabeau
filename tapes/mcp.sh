#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: mcp.sh [--no-prompt] [vhs.tape]" >&2
}

info() {
  echo "[mcp.sh] $*"
}

error() {
  echo "[mcp.sh] Error: $*" >&2
}

RESEARCH_FRIEND_DIR="${HOME}/.research-friend"
RESEARCH_FRIEND_BACKUP="${HOME}/.research-friend_original"
PRELOAD_URLS=(
  "https://arxiv.org/pdf/2601.22401v1.pdf"
)

backup_created=0
settings_captured=0
settings_restored=0
directories_restored=0
skip_dir_cleanup=0
waiting_for_input=0
no_prompt=0

original_enabled=off
original_yolo=off

restore_settings() {
  if [[ "${settings_captured}" -eq 0 || "${settings_restored}" -eq 1 ]]; then
    return
  fi

  info "Restoring MCP settings for research-friend (${original_enabled}, yolo ${original_yolo})..."
  set +e
  chabeau set mcp research-friend "${original_enabled}"
  chabeau set mcp research-friend yolo "${original_yolo}"
  set -e
  settings_restored=1
}

restore_directories() {
  if [[ "${directories_restored}" -eq 1 || "${skip_dir_cleanup}" -eq 1 ]]; then
    return
  fi

  info "Removing test MCP directory at ${RESEARCH_FRIEND_DIR}..."
  rm -rf "${RESEARCH_FRIEND_DIR}"

  if [[ "${backup_created}" -eq 1 && -e "${RESEARCH_FRIEND_BACKUP}" ]]; then
    info "Restoring original MCP directory from ${RESEARCH_FRIEND_BACKUP}..."
    mv "${RESEARCH_FRIEND_BACKUP}" "${RESEARCH_FRIEND_DIR}"
  fi

  directories_restored=1
}

handle_int_term() {
  if [[ "${waiting_for_input}" -eq 1 ]]; then
    skip_dir_cleanup=1
    info "Interrupted during cleanup prompt; leaving directory state unchanged."
  fi
  exit 130
}

handle_exit() {
  restore_settings
  restore_directories
}

trap handle_exit EXIT
trap handle_int_term INT TERM

if [[ $# -lt 1 ]]; then
  error "Missing tape argument."
  usage
  exit 1
fi

tape_file=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-prompt)
      no_prompt=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      error "Unknown option: $1"
      usage
      exit 1
      ;;
    *)
      if [[ -n "${tape_file}" ]]; then
        error "Too many positional arguments."
        usage
        exit 1
      fi
      tape_file="$1"
      ;;
  esac
  shift
done

if [[ -z "${tape_file}" ]]; then
  error "Missing tape argument."
  usage
  exit 1
fi

info "Preparing MCP test environment for tape: ${tape_file}"

if [[ -e "${RESEARCH_FRIEND_BACKUP}" ]]; then
  error "Backup path already exists: ${RESEARCH_FRIEND_BACKUP}"
  error "Please restore or remove it before running this script."
  exit 1
fi

if [[ -e "${RESEARCH_FRIEND_DIR}" ]]; then
  info "Backing up existing MCP directory to ${RESEARCH_FRIEND_BACKUP}..."
  mv "${RESEARCH_FRIEND_DIR}" "${RESEARCH_FRIEND_BACKUP}"
  backup_created=1
else
  info "No existing ${RESEARCH_FRIEND_DIR} directory found; continuing."
fi

info "Reading current Chabeau settings..."
set_output="$(chabeau set)"
research_friend_line="$(
  printf '%s\n' "${set_output}" | sed -n 's/^[[:space:]]*research-friend:[[:space:]]*//p' | head -n 1
)"

if [[ -z "${research_friend_line}" ]]; then
  error "Could not find a research-friend entry in \`chabeau set\` output."
  error "Ensure the research-friend MCP server is configured before recording."
  exit 1
fi

if [[ "${research_friend_line}" == on* ]]; then
  original_enabled=on
else
  original_enabled=off
fi

if [[ "${research_friend_line}" == *"[yolo]"* ]]; then
  original_yolo=on
else
  original_yolo=off
fi

settings_captured=1
info "Captured settings: research-friend ${original_enabled}, yolo ${original_yolo}."

info "Enabling research-friend MCP and yolo approvals for recording..."
chabeau set mcp research-friend on
chabeau set mcp research-friend yolo on

inbox_dir="${RESEARCH_FRIEND_DIR}/inbox"
info "Preparing research-friend inbox at ${inbox_dir}..."
mkdir -p "${inbox_dir}"

for preload_url in "${PRELOAD_URLS[@]}"; do
  target_name="$(basename "${preload_url}")"
  target_path="${inbox_dir}/${target_name}"
  info "Downloading ${preload_url} to ${target_path}..."
  curl -fsSL "${preload_url}" -o "${target_path}"
done

info "Running VHS tape: ${tape_file}"
set +e
vhs "${tape_file}"
vhs_status=$?
set -e

restore_settings

info "VHS exited with status ${vhs_status}."
if [[ "${no_prompt}" -eq 1 ]]; then
  info "--no-prompt enabled; cleaning up immediately."
else
  info "Press Enter to clean up and restore ${RESEARCH_FRIEND_DIR} (Ctrl+C to leave current state)."
  waiting_for_input=1
  read -r _
  waiting_for_input=0
fi

restore_directories
info "MCP environment restore complete."

exit "${vhs_status}"
