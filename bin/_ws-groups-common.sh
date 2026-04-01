# Common helper functions for workspace group scripts.
# Source this file from other scripts in the same directory.

_WS_GROUPS_SCRIPT_DIR="$(cd -- "$(dirname "${BASH_SOURCE[1]}")" && pwd -P)"

command_exists() {
  type "$1" &> /dev/null
}

get_tool() {
  local filename="$1"
  local tool="${_WS_GROUPS_SCRIPT_DIR}/${filename}"
  command_exists "${tool}" || tool="${filename}"
  printf '%s\n' "${tool}"
}

# Send a command to the running window manager (i3-msg or swaymsg).
wm_msg() {
  local tool
  tool="$(get_tool "${I3_WORKSPACE_GROUPS_CLI:-i3sets-client}")"
  "${tool}" wm-msg "$@"
}

# Menu command array — override with WS_GROUPS_MENU_CMD env var.
_MENU_CMD="${WS_GROUPS_MENU_CMD:-rofi}"
