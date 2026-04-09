# i3 Workspace Sets

A Python library and set of command line tools for managing i3wm workspaces in
sets. I find this tool useful for managing many workspaces in i3.

[![PyPI version](https://badge.fury.io/py/i3-workspace-groups.svg)](https://badge.fury.io/py/i3-workspace-groups)
[![pipeline status](https://gitlab.com/infokiller/i3-workspace-groups/badges/master/pipeline.svg)](https://gitlab.com/infokiller/i3-workspace-groups/commits/master)

![Demo flow](./assets/demo.gif?raw=true)

## Table of Contents

- [Background](#background)
- [Installation](#installation)
- [Configuration](#configuration)
  - [i3](#i3)
  - [i3-workspace-sets](#i3-workspace-sets)
- [Usage](#usage)
  - [Example walk through](#example-walk-through)
- [Concepts](#concepts)
  - [Active workspace](#active-workspace)
  - [Active set](#active-set)
  - [Focused set](#focused-set)
  - [Default set](#default-set)
- [Limitations](#limitations)
  - [Sway compatibility](#sway-compatibility)
  - [Environment variables](#environment-variables)
  - [Polybar](#polybar)
  - [Waybar](#waybar)

## Background

I often find myself working with many i3 workspaces at once (7-8+), usually
related to multiple projects/contexts (personal/work etc). This has caused me a
few issues, for example:

- Working with a set of workspaces of a given project/context, without being
  distracted by unrelated workspaces.
- Reusing the same workspace number in multiple projects/contexts. For example,
  I have two different emails for personal and work stuff, and I want `Super+1`
  to always switch to the workspace with the email client relevant to my current
  context.
- Finding a free workspace for a new window (that can also be reached with my
  keybindings)

This has led me to create the
[i3-workspace-sets](https://github.com/infokiller/i3-workspace-groups)
project, which enables you to define and manage sets of workspaces, each with
their own "namespace", and switch between them.

## Installation

The scripts can be installed using pip:

```shell
python3 -m pip install i3-workspace-groups
```

Then you should be able to run the command line tool
[`i3-workspace-sets`](bin/i3-workspace-sets). There are also a few utility
scripts provided that require [rofi](https://github.com/DaveDavenport/rofi) and
which are useful for interactively managing the sets, using rofi as the UI.
They include:

- [`i3-assign-workspace-to-set`](bin/i3-assign-workspace-to-set)
- [`i3-focus-on-workspace`](bin/i3-focus-on-workspace)
- [`i3-move-to-workspace`](bin/i3-move-to-workspace)
- [`i3-rename-workspace`](bin/i3-rename-workspace)
- [`i3-switch-active-workspace-set`](bin/i3-switch-active-workspace-set)

If you want to use client/server mode for improved speed/latency, it's
recommended to install one of the following tools to further improve speed:

- [socat](http://www.dest-unreach.org/socat/): available in all major distros
- BSD netcat (GNU version not supported)
- [ncat](https://nmap.org/ncat/)

## Configuration

### i3

In order to use these tools effectively, commands need to be bound to
keybindings. For example, my i3 config contains the following exerts:

<!-- markdownlint-disable fenced-code-language -->

```ini
set $mod Mod4

set $exec_i3_sets exec --no-startup-id i3-workspace-sets

# Switch active workspace set
bindsym $mod+g exec --no-startup-id i3-switch-active-workspace-set

# Assign workspace to a set
bindsym $mod+Shift+g exec --no-startup-id i3-assign-workspace-to-set

# Select workspace to focus on
bindsym $mod+w exec --no-startup-id i3-focus-on-workspace

# Move the focused container to another workspace
bindsym $mod+Shift+w exec --no-startup-id i3-move-to-workspace

# Rename/renumber workspace. Uses Super+Alt+n
bindsym Mod1+Mod4+n exec --no-startup-id i3-rename-workspace

bindsym $mod+1 $exec_i3_sets workspace-number 1
bindsym $mod+2 $exec_i3_sets workspace-number 2
bindsym $mod+3 $exec_i3_sets workspace-number 3
bindsym $mod+4 $exec_i3_sets workspace-number 4
bindsym $mod+5 $exec_i3_sets workspace-number 5
bindsym $mod+6 $exec_i3_sets workspace-number 6
bindsym $mod+7 $exec_i3_sets workspace-number 7
bindsym $mod+8 $exec_i3_sets workspace-number 8
bindsym $mod+9 $exec_i3_sets workspace-number 9
bindsym $mod+0 $exec_i3_sets workspace-number 10

bindsym $mod+Shift+1 $exec_i3_sets move-to-number 1
bindsym $mod+Shift+2 $exec_i3_sets move-to-number 2
bindsym $mod+Shift+3 $exec_i3_sets move-to-number 3
bindsym $mod+Shift+4 $exec_i3_sets move-to-number 4
bindsym $mod+Shift+5 $exec_i3_sets move-to-number 5
bindsym $mod+Shift+6 $exec_i3_sets move-to-number 6
bindsym $mod+Shift+7 $exec_i3_sets move-to-number 7
bindsym $mod+Shift+8 $exec_i3_sets move-to-number 8
bindsym $mod+Shift+9 $exec_i3_sets move-to-number 9
bindsym $mod+Shift+0 $exec_i3_sets move-to-number 10

# Switch to previous/next workspace (in all sets).
bindsym $mod+p workspace prev
bindsym $mod+n workspace next

bar {
  strip_workspace_numbers yes
  # The rest of your bar config goes below.
  # ...
}
```

### i3-workspace-sets

i3-workspace-sets has an optional config file located at
`$XDG_CONFIG_HOME/i3-workspace-sets/config.toml` (defaults to
`~/.config/i3-workspace-sets/config.toml`). See the
[default config file](./i3wsgroups/default_config.toml) for all the possible
options to configure, their meaning, and their default values.

## Usage

The main operations the CLI tool `i3-workspace-sets` supports are:

- Assign the focused workspace to a set with a given name (and creating the
  set if it doesn't exist).
- Switch the currently [active set](#active-set). Note that the active set
  is not necessarily the same as the [focused set](#focused-set).
- Navigation and movement within a set while ignoring the other sets. See
  examples below.

The tools provided use i3 workspace names to store and read the set for each
workspace. For example, if a user assigns the workspace `mail` to the set
`work`, it will be renamed to `work:mail`.

### Example walk through

> **NOTE:** This walk through assumes that you configured keybindings like the
> [example i3 config](#i3).

Say we start with the following workspace names:

1. `1` with cat videos from YouTube.
2. `2` with a news reader.
3. `3` with a photo editor.
4. `4` with an email client for work.

An important thing to understand here is that every i3 workspace is always
assigned to a single set. And since we haven't assigned any workspace to a
set yet, all the workspaces are implicitly in the
[default set](#default-set), which is denoted as `<default>`.

After a few hours of leisure time, you decide to do some work, which requires
opening a few windows on a few workspaces. In order to create a new set, first
you switch to the workspace `4`, and then you press `Super+Shift+g`, which will
prompt you for a set to assign to the current workspace. You type `work` and
press enter. Since there's no set named `work` yet, the tool will create it
and assign the focused workspace to it. You will then notice that the workspace
name will change in i3bar to `work:4`. Then, you press `Super+g` in order to
switch the [active set](#active-set). You will be shown a list of existing
sets, which will now be `work` and `<default>`. You should now see your
workspaces in i3bar ordered as following: `work:4`, `1`, `2`, `3`. What happened
here? When you switched to the `work` set, the first thing that the tool did
was to move all the workspaces in the work set (only `work:mail`) to be in the
beginning of the workspace list. Then, it renamed the workspaces in the default
set to include the set name, so that they can be differentiated from other
workspaces in the `work` set with the same name.

Then, you decide that you want to open a new terminal window in a new workspace.
So you press `Super+2`, which will move you to a new workspace named `work:2`.
Note that although there is already a workspace with the name `2` in the default
set (now shown as `2` in the workspace list), using `Super+2` actually takes
you to a new empty workspace in the set `work`.

After some time working, you become lazy and you want to get back to cat videos,
but you promise yourself to get back to work in a few hours, and you don't want
to lose your open windows. So you press `Super+g` to switch the active work back
to the default one. You should now see your workspaces in i3bar ordered as
following: `1`, `2`, `3`, `work:4`. The focus will also shift to the first
workspace in the default set (`1` in this case). Now that you're back in the
default set, pressing `Super+2` will again lead you to the workspace `2` in
the default set.

## Concepts

### Active workspace

The active workspace is the workspace with the lowest number. Typically, this
will be the workspace that appears first in the workspace list in i3bar (the
leftmost one).

> **NOTE:** In a multi-monitor setup, there is an active workspace per monitor.
>
> **NOTE:** The active workspace is not necessarily the focused workspace.

### Active set

The active set is the set of the [active workspace](#active-workspace). This
set will normally contain workspaces related to the task you're doing at the
time it's active. When you want to work on another task, you can switch the
active set. Workspaces that are not in the active set can still be
interacted with, but some commands provided are designed to make it easier to
interact with the workspaces of the active set.

> **NOTE:** In a multi-monitor setup, there is an active set per monitor.

### Focused set

The set of the focused workspace.

### Default set

The set of workspaces that were not assigned to a set by the user. This
set is displayed as `<default>`. When you start using i3-workspace-sets,
none of your current workspaces will be assigned to a set yet, so they will
all be in the default set.

## Limitations

- **Interaction with other i3 tools**: workspace names are used for storing the
  set, so if another tool changes a workspace name without preserving the
  format that i3-workspace-sets uses, i3-workspace-sets can make a mistake
  about the set assignment.
- ~~**Latency**: there can be noticeable latency in some machines for the script
  commands. On my high performance desktop this is not noticeable, but on my
  laptop it is. I measured the latency of commands to be around 100-200 ms, most
  of it coming from importing python libraries, so it's not possible to reduce
  it much without running it as a daemon (which will overcomplicate things). In
  the long term, I plan to rewrite it in go.~~ **UPDATE**: there is a new
  experimental client/server mode which significantly reduces latency.
  Documentation is still WIP (see
  <https://github.com/infokiller/i3-workspace-groups/issues/52>).
- **Number of monitors/sets/workspaces**: Supports up to 10 monitors, each
  containing up to 100 sets, each containing up to 100 workspaces.

### Sway compatibility

This project works on both i3 and sway. The core library uses
[i3ipc](https://github.com/acrisci/i3ipc-python) which supports both window
managers, and the tools auto-detect which WM is running (via `$SWAYSOCK` /
`$I3SOCK`). Your i3 config keybindings can be copied directly to a sway config
with no changes — the same commands work on both.

The `i3sets-client` binary handles WM detection for:

- **Socket path**: Uses `$WAYLAND_DISPLAY` on sway, `$DISPLAY` on i3
- **WM commands**: `i3sets-client wm-msg <args>` runs `swaymsg` or `i3-msg`
  as appropriate
- **WM detection**: `i3sets-client detect-wm` prints `sway` or `i3`

### Environment variables

| Variable | Description | Default |
|----------|-------------|---------|
| `I3_WORKSPACE_SETS_SOCKET` | Override the Unix socket path | Auto-detected |
| `I3_WORKSPACE_SETS_CLI` | Override the client binary name | `i3sets-client` |
| `WS_SETS_MENU_CMD` | Override the menu/launcher binary (e.g. `wofi`, `fuzzel`, `bemenu`) | `rofi` |

### Polybar

The official `internal/i3` module does not support workspace sets.

In order to display workspace information in polybar, there are two steps:

1. Add the custom i3 workspace sets module to your polybar
2. Run a script in the background to update polybar's display whenever an i3
   window event occurs

#### 1. Add the custom i3 workspace sets module to your polybar config

Create an `i3-mod` module by adding the following to your polybar config:

```
[module/i3-mod]
type = custom/ipc
hook-0 = ${env:I3_MOD_HOOK}
initial = 1
```

Then, add the `i3-mod` module to your modules:

```
modules-center = i3-mod
```

Then, when launching polybar, do something like the following to configure the
`I3_MOD_HOOK`:

```bash
while IFS='' read -r monitor; do
    i3_mod_hook="i3-workspace-sets polybar-hook --monitor '${monitor}'"
    I3_MOD_HOOK="${i3_mod_hook}" polybar your-bar-name &
done < <(polybar --list-monitors | cut -d':' -f1)
```

#### 2. Run a background script to update polybar's on i3 events

Run the
[i3-sets-polybar-module-updater](./bin/i3-sets-polybar-module-updater)
script. This script is responsible for calling the hook to update polybar
whenever a relevant i3 window event occurs.

### Waybar

For sway users (or i3 users running waybar), a waybar custom module is provided.

#### 1. Add the custom module to your waybar config

```json
"custom/workspace-sets": {
    "exec": "i3-sets-waybar-module",
    "return-type": "json",
    "interval": "once",
    "signal": 8,
    "on-click": "i3-switch-active-workspace-set"
}
```

Then add `"custom/workspace-sets"` to your bar modules.

#### 2. Run a background script to update waybar on workspace events

Run `i3-sets-bar-module-updater --bar waybar` (or just
`i3-sets-bar-module-updater` which auto-detects the bar). This sends
`SIGRTMIN+8` to waybar whenever a workspace event occurs, triggering the custom
module to refresh. You can change the signal number with `--waybar-signal N`
(must match the `signal` field in your waybar config).
