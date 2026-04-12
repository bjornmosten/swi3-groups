# Common helper functions for swi3-groups scripts.
# Source this file from other scripts in the same directory.

_SWIGROUPSDIR="$(cd -- "$(dirname "${BASH_SOURCE[1]}")" && pwd -P)"

command_exists() {
  type "$1" &> /dev/null
}

get_tool() {
  local filename="$1"
  local tool="${_SWIGROUPSDIR}/${filename}"
  command_exists "${tool}" || tool="${filename}"
  printf '%s\n' "${tool}"
}

# Send a command to the running window manager (i3-msg or swaymsg).
wm_msg() {
  local tool
  tool="$(get_tool "${SWI3GROUPS_CLI:-swi3-groups-client}")"
  "${tool}" wm-msg "$@"
}

# Auto-detect menu backend.
# Probe order: rofi → wofi → dmenu.
# Override with SWI3GROUPS_MENU env var.
if [[ -n "${SWI3GROUPS_MENU:-}" ]]; then
  _MENU_CMD="${SWI3GROUPS_MENU}"
elif command_exists rofi; then
  _MENU_CMD="rofi"
elif command_exists wofi; then
  _MENU_CMD="wofi"
elif command_exists fuzzel; then
  _MENU_CMD="fuzzel"
elif command_exists dmenu; then
  _MENU_CMD="dmenu"
else
  _MENU_CMD=""
fi

# Build a dmenu invocation into the global _MENU_ARGS array.
# Usage: _build_menu_cmd PROMPT [MESG [THEME_STR]]
# MESG and THEME_STR are only supported on rofi.
_build_menu_cmd() {
  local prompt="${1:-}"
  local mesg="${2:-}"
  local theme="${3:-}"
  case "${_MENU_CMD}" in
    rofi)
      local focused_output=""
      if command_exists swaymsg && command_exists jq; then
        focused_output="$(swaymsg -t get_outputs 2> /dev/null |
          jq -r '.[] | select(.focused) | .name' 2> /dev/null || true)"
      elif command_exists i3-msg && command_exists jq; then
        focused_output="$(i3-msg -t get_workspaces 2> /dev/null |
          jq -r '.[] | select(.focused) | .output' 2> /dev/null || true)"
      fi
      _MENU_ARGS=("rofi" "-dmenu" "-p" "${prompt}" "-kb-cancel" "Escape")
      if [[ -n "${focused_output}" ]]; then
        _MENU_ARGS+=(-monitor "${focused_output}")
      fi
      if [[ -n "${theme}" ]]; then
        _MENU_ARGS+=(-theme-str "${theme}")
      fi
      if [[ -n "${mesg}" ]]; then
        _MENU_ARGS+=(-mesg "${mesg}")
      fi
      ;;
    wofi)
      _MENU_ARGS=("wofi" "--dmenu" "--prompt" "${prompt}")
      ;;
    dmenu)
      _MENU_ARGS=("dmenu" "-p" "${prompt}")
      ;;
    "")
      echo "swi3-groups: no menu launcher found (install rofi, wofi, or dmenu; or set SWI3GROUPS_MENU)" >&2
      _MENU_ARGS=("false")
      ;;
    *)
      # User-supplied custom command — pass it as-is.
      # shellcheck disable=SC2086
      read -ra _MENU_ARGS <<< "${_MENU_CMD}"
      ;;
  esac
}
