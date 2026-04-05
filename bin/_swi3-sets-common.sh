# Common helper functions for swi3-sets scripts.
# Source this file from other scripts in the same directory.

_SWISETSDIR="$(cd -- "$(dirname "${BASH_SOURCE[1]}")" && pwd -P)"

command_exists() {
  type "$1" &> /dev/null
}

get_tool() {
  local filename="$1"
  local tool="${_SWISETSDIR}/${filename}"
  command_exists "${tool}" || tool="${filename}"
  printf '%s\n' "${tool}"
}

# Send a command to the running window manager (i3-msg or swaymsg).
wm_msg() {
  local tool
  tool="$(get_tool "${SWI3SETS_CLI:-swi3-sets-client}")"
  "${tool}" wm-msg "$@"
}

# Auto-detect menu backend: wofi on Wayland, rofi on X11.
# Override with SWI3SETS_MENU env var.
if [[ -n "${SWI3SETS_MENU:-}" ]]; then
  _MENU_CMD="${SWI3SETS_MENU}"
elif [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
  _MENU_CMD="wofi"
else
  _MENU_CMD="rofi"
fi

# Build a dmenu invocation into the global _MENU_ARGS array.
# Usage: _build_menu_cmd PROMPT [MESG [THEME_STR]]
# MESG and THEME_STR are silently ignored on wofi.
_build_menu_cmd() {
  local prompt="${1:-}"
  local mesg="${2:-}"
  local theme="${3:-}"
  if [[ "${_MENU_CMD}" == "wofi" ]]; then
    _MENU_ARGS=("wofi" "--dmenu" "--prompt" "${prompt}")
  else
    _MENU_ARGS=("rofi" "-dmenu" "-p" "${prompt}")
    [[ -n "${theme}" ]] && _MENU_ARGS+=(-theme-str "${theme}")
    [[ -n "${mesg}" ]]  && _MENU_ARGS+=(-mesg "${mesg}")
  fi
}
