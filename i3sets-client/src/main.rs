use std::collections::{BTreeMap, HashSet};
use std::env;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{self, Command};

use swayipc::{Connection, EventType, Fallible};

// ── workspace name format ────────────────────────────────────────────────────
// global_number:\u{200b}group\u{200b}:static_name\u{200b}:dynamic_name\u{200b}:local_number
const DELIM: char = '\u{200b}';
const MAX_GROUPS_PER_MONITOR: i64 = 1000;
const MAX_WORKSPACES_PER_GROUP: i64 = 100;

#[derive(Debug, Default, Clone)]
struct WsMeta {
    global_number: Option<i64>,
    group: Option<String>,
    static_name: Option<String>,
    dynamic_name: Option<String>,
    local_number: Option<i64>,
}

fn sanitize(s: &str) -> String {
    let s = s.replace(DELIM, "%");
    if s.starts_with(':') { s[1..].to_string() } else { s }
}

fn strip_suffix_colon(s: &str) -> &str {
    s.strip_suffix(':').unwrap_or(s)
}

fn strip_prefix_colon(s: &str) -> &str {
    s.strip_prefix(':').unwrap_or(s)
}

fn parse_name(name: &str) -> WsMeta {
    let parts: Vec<&str> = name.splitn(5, DELIM).collect();
    if parts.len() != 5 {
        return WsMeta {
            static_name: Some(sanitize(name)),
            group: Some(String::new()),
            ..Default::default()
        };
    }
    let global_number = if parts[0].is_empty() {
        None
    } else {
        strip_suffix_colon(parts[0]).parse::<i64>().ok()
    };
    let group = Some(if parts[1].is_empty() {
        String::new()
    } else {
        strip_suffix_colon(parts[1]).to_string()
    });
    let static_name = if parts[2].is_empty() {
        None
    } else {
        Some(strip_prefix_colon(parts[2]).to_string())
    };
    let dynamic_name = if parts[3].is_empty() {
        None
    } else {
        Some(strip_prefix_colon(parts[3]).to_string())
    };
    let local_number = if parts[4].is_empty() {
        None
    } else {
        strip_prefix_colon(parts[4]).parse::<i64>().ok()
    };
    WsMeta { global_number, group, static_name, dynamic_name, local_number }
}

fn create_name(m: &WsMeta) -> String {
    let gn = m.global_number.expect("global_number required");
    let group = m.group.as_deref().unwrap_or("");
    let mut need_prefix = !group.is_empty();

    let static_part = match m.static_name.as_deref() {
        Some(s) if !s.is_empty() => {
            let v = if need_prefix { format!(":{}", s) } else { s.to_string() };
            need_prefix = true;
            v
        }
        _ => String::new(),
    };
    let dynamic_part = match m.dynamic_name.as_deref() {
        Some(s) if !s.is_empty() => {
            let v = if need_prefix { format!(":{}", s) } else { s.to_string() };
            need_prefix = true;
            v
        }
        _ => String::new(),
    };
    let local_part = match m.local_number {
        Some(n) => {
            if need_prefix { format!(":{}", n) } else { n.to_string() }
        }
        None => String::new(),
    };

    format!(
        "{}:{d}{}{d}{}{d}{}{d}{}",
        gn, group, static_part, dynamic_part, local_part,
        d = DELIM
    )
}

fn compute_global_number(monitor_index: i64, group_index: i64, local_number: i64) -> i64 {
    monitor_index * (MAX_GROUPS_PER_MONITOR * MAX_WORKSPACES_PER_GROUP)
        + group_index * MAX_WORKSPACES_PER_GROUP
        + local_number
}

fn global_number_to_group_index(global_number: i64) -> i64 {
    global_number % (MAX_GROUPS_PER_MONITOR * MAX_WORKSPACES_PER_GROUP) / MAX_WORKSPACES_PER_GROUP
}

fn global_number_to_local_number(global_number: i64) -> i64 {
    global_number % MAX_WORKSPACES_PER_GROUP
}

fn get_local_number(meta: &WsMeta) -> Option<i64> {
    meta.local_number
        .or_else(|| meta.global_number.map(global_number_to_local_number))
}

// ── IPC helpers ──────────────────────────────────────────────────────────────

fn get_sorted_outputs(conn: &mut Connection) -> Fallible<Vec<swayipc::Output>> {
    let mut outputs: Vec<swayipc::Output> =
        conn.get_outputs()?.into_iter().filter(|o| o.active).collect();
    outputs.sort_by_key(|o| (o.rect.y, o.rect.x));
    Ok(outputs)
}

fn get_monitor_index(conn: &mut Connection, monitor_name: &str) -> Fallible<i64> {
    let outputs = get_sorted_outputs(conn)?;
    Ok(outputs.iter().position(|o| o.name == monitor_name).unwrap_or(0) as i64)
}

fn get_focused_monitor_name(conn: &mut Connection) -> Fallible<String> {
    let workspaces = conn.get_workspaces()?;
    workspaces
        .into_iter()
        .find(|ws| ws.focused)
        .map(|ws| ws.output)
        .ok_or_else(|| swayipc::Error::CommandFailed("no focused workspace".to_string()))
}

fn get_all_workspaces(conn: &mut Connection) -> Fallible<Vec<swayipc::Workspace>> {
    conn.get_workspaces()
}

fn get_monitor_workspaces(
    conn: &mut Connection,
    monitor_name: &str,
) -> Fallible<Vec<swayipc::Workspace>> {
    Ok(conn
        .get_workspaces()?
        .into_iter()
        .filter(|ws| ws.output == monitor_name)
        .collect())
}

fn get_monitor_to_workspaces(
    conn: &mut Connection,
) -> Fallible<BTreeMap<String, Vec<swayipc::Workspace>>> {
    let mut map: BTreeMap<String, Vec<swayipc::Workspace>> = BTreeMap::new();
    for ws in conn.get_workspaces()? {
        map.entry(ws.output.clone()).or_default().push(ws);
    }
    Ok(map)
}

fn get_focused_workspace(conn: &mut Connection) -> Fallible<swayipc::Workspace> {
    conn.get_workspaces()?
        .into_iter()
        .find(|ws| ws.focused)
        .ok_or_else(|| swayipc::Error::CommandFailed("no focused workspace".to_string()))
}

/// Ordered map: group → Vec<Workspace> in encounter order (preserves i3 order).
fn group_to_workspaces_ordered(
    workspaces: &[swayipc::Workspace],
) -> Vec<(String, Vec<swayipc::Workspace>)> {
    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, Vec<swayipc::Workspace>> = BTreeMap::new();
    for ws in workspaces {
        let group = parse_name(&ws.name).group.unwrap_or_default();
        if !map.contains_key(&group) {
            order.push(group.clone());
        }
        map.entry(group).or_default().push(ws.clone());
    }
    order
        .into_iter()
        .map(|g| {
            let v = map.remove(&g).unwrap();
            (g, v)
        })
        .collect()
}

fn send_i3_command(conn: &mut Connection, cmd: &str) -> Fallible<()> {
    let results = conn.run_command(cmd)?;
    for r in results {
        if let Err(e) = r {
            eprintln!("warn: command error: {}", e);
        }
    }
    Ok(())
}

fn focus_workspace(conn: &mut Connection, name: &str, auto_back_and_forth: bool) -> Fallible<()> {
    let opt = if auto_back_and_forth {
        ""
    } else {
        "--no-auto-back-and-forth "
    };
    send_i3_command(
        conn,
        &format!("workspace {}\"{}\"", opt, name.replace('"', "\\\"")),
    )
}

fn rename_workspace(conn: &mut Connection, old: &str, new: &str) -> Fallible<()> {
    if old == new {
        return Ok(());
    }
    send_i3_command(
        conn,
        &format!(
            "rename workspace \"{}\" to \"{}\"",
            old.replace('"', "\\\""),
            new.replace('"', "\\\"")
        ),
    )
}

// ── group index ───────────────────────────────────────────────────────────────

fn get_group_index(
    target_group: &str,
    groups: &[(String, Vec<swayipc::Workspace>)],
) -> i64 {
    let mut group_to_index: BTreeMap<String, i64> = BTreeMap::new();
    for (group, workspaces) in groups {
        for ws in workspaces {
            if let Some(gn) = parse_name(&ws.name).global_number {
                group_to_index
                    .entry(group.clone())
                    .or_insert_with(|| global_number_to_group_index(gn));
                break;
            }
        }
    }
    if let Some(&idx) = group_to_index.get(target_group) {
        return idx;
    }
    group_to_index.values().copied().max().map(|m| m + 1).unwrap_or(0)
}

// ── used/free local numbers ───────────────────────────────────────────────────

fn get_used_local_numbers(workspaces: &[swayipc::Workspace]) -> HashSet<i64> {
    workspaces
        .iter()
        .filter_map(|ws| parse_name(&ws.name).local_number)
        .collect()
}

fn get_lowest_free_local_numbers(n: usize, used: &HashSet<i64>) -> Vec<i64> {
    let mut result = Vec::new();
    let mut candidate = 1i64;
    while result.len() < n {
        if !used.contains(&candidate) {
            result.push(candidate);
        }
        candidate += 1;
    }
    result
}

fn find_free_local_number(conn: &mut Connection, target_group: &str) -> Fallible<i64> {
    let all = get_all_workspaces(conn)?;
    let groups = group_to_workspaces_ordered(&all);
    let group_workspaces: Vec<swayipc::Workspace> = groups
        .into_iter()
        .filter(|(g, _)| g == target_group)
        .flat_map(|(_, ws)| ws)
        .collect();
    let used = get_used_local_numbers(&group_workspaces);
    Ok(get_lowest_free_local_numbers(1, &used)[0])
}

// ── organize workspace groups (renumber) ──────────────────────────────────────

fn organize_workspace_groups(
    conn: &mut Connection,
    monitor_name: &str,
    ordered_groups: &[(String, Vec<swayipc::Workspace>)],
    all_workspaces: &[swayipc::Workspace],
) -> Fallible<()> {
    let monitor_index = get_monitor_index(conn, monitor_name)?;
    let all_groups = group_to_workspaces_ordered(all_workspaces);

    for (group_index, (group, workspaces)) in ordered_groups.iter().enumerate() {
        let monitor_ws_names: HashSet<String> =
            workspaces.iter().map(|ws| ws.name.clone()).collect();

        // Local numbers used by this group on OTHER monitors
        let other_group_ws: Vec<swayipc::Workspace> = all_groups
            .iter()
            .filter(|(g, _)| g == group)
            .flat_map(|(_, ws)| {
                ws.iter()
                    .filter(|w| !monitor_ws_names.contains(&w.name))
                    .cloned()
            })
            .collect();
        let used_in_others = get_used_local_numbers(&other_group_ws);

        let mut used_so_far = used_in_others.clone();
        for ws in workspaces {
            let meta = parse_name(&ws.name);
            let ln = if let Some(n) = meta.local_number {
                if !used_so_far.contains(&n) {
                    n
                } else {
                    get_lowest_free_local_numbers(1, &used_so_far)[0]
                }
            } else {
                get_lowest_free_local_numbers(1, &used_so_far)[0]
            };
            used_so_far.insert(ln);

            let mut new_meta = meta;
            new_meta.group = Some(group.clone());
            new_meta.local_number = Some(ln);
            new_meta.global_number = Some(compute_global_number(
                monitor_index,
                group_index as i64,
                ln,
            ));
            new_meta.dynamic_name = Some(String::new());
            let new_name = create_name(&new_meta);
            rename_workspace(conn, &ws.name, &new_name)?;
        }
    }
    Ok(())
}

// ── group context ─────────────────────────────────────────────────────────────

enum GroupContext {
    Active,
    Focused,
    Named(String),
    None,
}

fn resolve_group(conn: &mut Connection, ctx: &GroupContext) -> Fallible<String> {
    match ctx {
        GroupContext::Named(name) => Ok(name.clone()),
        GroupContext::Focused => {
            let ws = get_focused_workspace(conn)?;
            Ok(parse_name(&ws.name).group.unwrap_or_default())
        }
        GroupContext::Active | GroupContext::None => {
            let monitor = get_focused_monitor_name(conn)?;
            let ws = get_monitor_workspaces(conn, &monitor)?;
            let groups = group_to_workspaces_ordered(&ws);
            Ok(groups.into_iter().next().map(|(g, _)| g).unwrap_or_default())
        }
    }
}

// ── workspace-by-local-number ─────────────────────────────────────────────────

fn create_workspace_name_for(
    conn: &mut Connection,
    group: &str,
    local_number: i64,
) -> Fallible<String> {
    let monitor = get_focused_monitor_name(conn)?;
    let monitor_index = get_monitor_index(conn, &monitor)?;
    let monitor_ws = get_monitor_workspaces(conn, &monitor)?;
    let groups = group_to_workspaces_ordered(&monitor_ws);
    let group_index = get_group_index(group, &groups);
    let gn = compute_global_number(monitor_index, group_index, local_number);
    let meta = WsMeta {
        global_number: Some(gn),
        group: Some(group.to_string()),
        local_number: Some(local_number),
        ..Default::default()
    };
    Ok(create_name(&meta))
}

fn get_workspace_by_local_number(
    conn: &mut Connection,
    group: &str,
    local_number: i64,
) -> Fallible<(String, bool)> {
    let all = get_all_workspaces(conn)?;
    for ws in &all {
        let meta = parse_name(&ws.name);
        if meta.group.as_deref() == Some(group)
            && get_local_number(&meta) == Some(local_number)
        {
            return Ok((ws.name.clone(), true));
        }
    }
    Ok((create_workspace_name_for(conn, group, local_number)?, false))
}

// ── command handlers ──────────────────────────────────────────────────────────

fn cmd_list_groups(conn: &mut Connection, focused_monitor_only: bool) -> Fallible<String> {
    let workspaces = if focused_monitor_only {
        let monitor = get_focused_monitor_name(conn)?;
        get_monitor_workspaces(conn, &monitor)?
    } else {
        get_all_workspaces(conn)?
    };
    let groups = group_to_workspaces_ordered(&workspaces);
    Ok(groups.into_iter().map(|(g, _)| g).collect::<Vec<_>>().join("\n"))
}

fn cmd_list_workspaces(
    conn: &mut Connection,
    fields: &[&str],
    focused_only: bool,
    focused_monitor_only: bool,
    group_ctx: &GroupContext,
) -> Fallible<String> {
    let workspaces = if focused_monitor_only {
        let monitor = get_focused_monitor_name(conn)?;
        get_monitor_workspaces(conn, &monitor)?
    } else {
        get_all_workspaces(conn)?
    };

    let target_workspaces: Vec<swayipc::Workspace> = match group_ctx {
        GroupContext::None => workspaces,
        _ => {
            let target_group = resolve_group(conn, group_ctx)?;
            let groups = group_to_workspaces_ordered(&workspaces);
            groups
                .into_iter()
                .filter(|(g, _)| *g == target_group)
                .flat_map(|(_, ws)| ws)
                .collect()
        }
    };

    let focused_ws_name = get_focused_workspace(conn).ok().map(|ws| ws.name);

    let filtered: Vec<swayipc::Workspace> = if focused_only {
        target_workspaces
            .into_iter()
            .filter(|ws| Some(&ws.name) == focused_ws_name.as_ref())
            .collect()
    } else {
        target_workspaces
    };

    let rows: Vec<String> = filtered
        .iter()
        .map(|ws| {
            let meta = parse_name(&ws.name);
            fields
                .iter()
                .map(|field| match *field {
                    "global_number" => meta
                        .global_number
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    "group" => meta.group.clone().unwrap_or_default(),
                    "static_name" => meta.static_name.clone().unwrap_or_default(),
                    "dynamic_name" => meta.dynamic_name.clone().unwrap_or_default(),
                    "local_number" => get_local_number(&meta)
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    "global_name" => ws.name.clone(),
                    "monitor" => ws.output.clone(),
                    "focused" => if ws.focused { "1".to_string() } else { "0".to_string() },
                    "window_icons" => String::new(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();

    Ok(rows.join("\n"))
}

fn cmd_workspace_number(
    conn: &mut Connection,
    local_number: i64,
    group_ctx: &GroupContext,
    auto_back_and_forth: bool,
) -> Fallible<String> {
    let group = resolve_group(conn, group_ctx)?;
    let (name, _) = get_workspace_by_local_number(conn, &group, local_number)?;
    focus_workspace(conn, &name, auto_back_and_forth)?;
    Ok(String::new())
}

fn cmd_workspace_relative(conn: &mut Connection, offset: i64) -> Fallible<String> {
    let focused = get_focused_workspace(conn)?;
    let focused_group = parse_name(&focused.name).group.unwrap_or_default();
    let all = get_all_workspaces(conn)?;
    let groups = group_to_workspaces_ordered(&all);
    let group_ws: Vec<swayipc::Workspace> = groups
        .into_iter()
        .filter(|(g, _)| *g == focused_group)
        .flat_map(|(_, ws)| ws)
        .collect();
    if group_ws.is_empty() {
        return Ok(String::new());
    }
    let current = group_ws
        .iter()
        .position(|ws| ws.name == focused.name)
        .unwrap_or(0);
    let next =
        ((current as i64 + offset).rem_euclid(group_ws.len() as i64)) as usize;
    focus_workspace(conn, &group_ws[next].name, false)?;
    Ok(String::new())
}

fn cmd_workspace_new(conn: &mut Connection, group_ctx: &GroupContext) -> Fallible<String> {
    let group = resolve_group(conn, group_ctx)?;
    let ln = find_free_local_number(conn, &group)?;
    cmd_workspace_number(conn, ln, group_ctx, true)
}

fn cmd_move_to_number(
    conn: &mut Connection,
    local_number: i64,
    group_ctx: &GroupContext,
    no_auto_back_and_forth: bool,
) -> Fallible<String> {
    let group = resolve_group(conn, group_ctx)?;
    let (name, _) = get_workspace_by_local_number(conn, &group, local_number)?;
    let flag = if no_auto_back_and_forth {
        "--no-auto-back-and-forth "
    } else {
        ""
    };
    send_i3_command(
        conn,
        &format!(
            "move {}container to workspace \"{}\"",
            flag,
            name.replace('"', "\\\"")
        ),
    )?;
    Ok(String::new())
}

fn cmd_move_relative(conn: &mut Connection, offset: i64) -> Fallible<String> {
    let focused = get_focused_workspace(conn)?;
    let focused_group = parse_name(&focused.name).group.unwrap_or_default();
    let all = get_all_workspaces(conn)?;
    let groups = group_to_workspaces_ordered(&all);
    let group_ws: Vec<swayipc::Workspace> = groups
        .into_iter()
        .filter(|(g, _)| *g == focused_group)
        .flat_map(|(_, ws)| ws)
        .collect();
    if group_ws.is_empty() {
        return Ok(String::new());
    }
    let current = group_ws
        .iter()
        .position(|ws| ws.name == focused.name)
        .unwrap_or(0);
    let next =
        ((current as i64 + offset).rem_euclid(group_ws.len() as i64)) as usize;
    send_i3_command(
        conn,
        &format!(
            "move container to workspace \"{}\"",
            group_ws[next].name.replace('"', "\\\"")
        ),
    )?;
    Ok(String::new())
}

fn cmd_move_to_new(conn: &mut Connection, group_ctx: &GroupContext) -> Fallible<String> {
    let group = resolve_group(conn, group_ctx)?;
    let ln = find_free_local_number(conn, &group)?;
    cmd_move_to_number(conn, ln, group_ctx, false)
}

fn cmd_switch_active_group(
    conn: &mut Connection,
    target_group: &str,
    focused_monitor_only: bool,
) -> Fallible<String> {
    let focused_monitor = get_focused_monitor_name(conn)?;
    let monitor_to_ws = get_monitor_to_workspaces(conn)?;

    for (monitor, workspaces) in &monitor_to_ws {
        let groups = group_to_workspaces_ordered(workspaces);
        let group_exists = groups.iter().any(|(g, _)| g == target_group);
        if monitor != &focused_monitor && (focused_monitor_only || !group_exists) {
            continue;
        }

        let mut reordered: Vec<(String, Vec<swayipc::Workspace>)> = Vec::new();
        let target_ws: Vec<swayipc::Workspace> = groups
            .iter()
            .filter(|(g, _)| g == target_group)
            .flat_map(|(_, ws)| ws.clone())
            .collect();
        reordered.push((target_group.to_string(), target_ws));
        for (g, ws) in &groups {
            if g != target_group {
                reordered.push((g.clone(), ws.clone()));
            }
        }

        let all_ws = get_all_workspaces(conn)?;
        organize_workspace_groups(conn, monitor, &reordered, &all_ws)?;
    }

    // Switch focus if needed — re-fetch after renames
    let focused_ws = get_focused_workspace(conn)?;
    let focused_group = parse_name(&focused_ws.name).group.unwrap_or_default();
    if focused_group == target_group {
        return Ok(String::new());
    }

    let monitor_ws = get_monitor_workspaces(conn, &focused_monitor)?;
    let groups = group_to_workspaces_ordered(&monitor_ws);
    let workspace_name = groups
        .iter()
        .find(|(g, _)| g == target_group)
        .and_then(|(_, ws)| ws.first().map(|w| w.name.clone()));

    let name = if let Some(n) = workspace_name {
        n
    } else {
        let monitor_index = get_monitor_index(conn, &focused_monitor)?;
        let groups2 = group_to_workspaces_ordered(&monitor_ws);
        let group_index = get_group_index(target_group, &groups2);
        let ln = find_free_local_number(conn, target_group)?;
        let gn = compute_global_number(monitor_index, group_index, ln);
        create_name(&WsMeta {
            global_number: Some(gn),
            group: Some(target_group.to_string()),
            local_number: Some(ln),
            ..Default::default()
        })
    };

    focus_workspace(conn, &name, false)?;
    Ok(String::new())
}

fn cmd_rename_workspace(
    conn: &mut Connection,
    new_name: Option<&str>,
    number: Option<i64>,
    group: Option<&str>,
) -> Fallible<String> {
    let focused = get_focused_workspace(conn)?;
    let mut meta = parse_name(&focused.name);
    if let Some(n) = new_name {
        meta.static_name = Some(n.to_string());
    }
    if let Some(n) = number {
        meta.local_number = Some(n);
    }
    if let Some(g) = group {
        meta.group = Some(g.to_string());
    }

    let g = meta.group.clone().unwrap_or_default();
    let ln = meta.local_number.unwrap_or(1);

    // Check for conflict with another workspace
    let all = get_all_workspaces(conn)?;
    let groups = group_to_workspaces_ordered(&all);
    let conflict = groups
        .iter()
        .filter(|(grp, _)| grp == &g)
        .flat_map(|(_, ws)| ws)
        .find(|ws| {
            ws.name != focused.name
                && get_local_number(&parse_name(&ws.name)) == Some(ln)
        })
        .is_some();

    if conflict {
        let used: HashSet<i64> = groups
            .iter()
            .filter(|(grp, _)| grp == &g)
            .flat_map(|(_, ws)| ws)
            .filter_map(|ws| parse_name(&ws.name).local_number)
            .collect();
        meta.local_number = Some(get_lowest_free_local_numbers(1, &used)[0]);
    }

    let monitor = get_focused_monitor_name(conn)?;
    let monitor_index = get_monitor_index(conn, &monitor)?;
    let monitor_ws = get_monitor_workspaces(conn, &monitor)?;
    let groups2 = group_to_workspaces_ordered(&monitor_ws);
    let group_index = get_group_index(&g, &groups2);
    let local_number = meta.local_number.unwrap_or(1);
    meta.global_number = Some(compute_global_number(monitor_index, group_index, local_number));
    meta.dynamic_name = Some(String::new());

    let new_ws_name = create_name(&meta);
    rename_workspace(conn, &focused.name, &new_ws_name)?;
    Ok(String::new())
}

fn cmd_assign_workspace_to_group(conn: &mut Connection, group: &str) -> Fallible<String> {
    cmd_rename_workspace(conn, None, None, Some(group))
}

fn cmd_waybar(conn: &mut Connection) -> Fallible<String> {
    let workspaces = get_all_workspaces(conn)?;

    let current_group = workspaces
        .iter()
        .find(|ws| ws.focused)
        .map(|ws| parse_name(&ws.name).group.unwrap_or_default())
        .unwrap_or_default();

    let groups = group_to_workspaces_ordered(&workspaces);

    let mut text_parts: Vec<String> = Vec::new();
    let mut tooltip_lines: Vec<String> = Vec::new();

    for (group, group_ws) in &groups {
        let display = if group.is_empty() { "default" } else { group.as_str() };

        if *group == current_group {
            let tabs: String = group_ws
                .iter()
                .map(|ws| {
                    let local_num = get_local_number(&parse_name(&ws.name)).unwrap_or(0);
                    if ws.focused {
                        format!(" [{}]", local_num)
                    } else {
                        format!(" {}", local_num)
                    }
                })
                .collect();
            text_parts.push(format!("{}:{}", display, tabs));
        } else {
            text_parts.push(display.to_string());
        }

        for ws in group_ws {
            let meta = parse_name(&ws.name);
            let local_num = get_local_number(&meta).unwrap_or(0);
            let mut label = format!("{}:{}", display, local_num);
            if let Some(ref name) = meta.static_name {
                if !name.is_empty() {
                    label.push_str(&format!(" {}", name));
                }
            }
            if ws.focused {
                label.push_str(" *");
            }
            tooltip_lines.push(label);
        }
    }

    let text = text_parts.join(" | ").replace('"', "\\\"");
    let tooltip = tooltip_lines.join("\\n").replace('"', "\\\"");
    let class = if current_group.is_empty() {
        "default".to_string()
    } else {
        current_group
    };

    Ok(format!(
        r#"{{"text": "{}", "tooltip": "{}", "class": "{}"}}"#,
        text, tooltip, class
    ))
}

fn run_bar_updater(signal: u32) {
    let subs = [
        EventType::Workspace,
    ];

    let events = Connection::new()
        .expect("could not connect to sway/i3")
        .subscribe(subs)
        .expect("could not subscribe to workspace events");

    for event in events {
        match event {
            Ok(_) => {
                Command::new("pkill")
                    .args([&format!("-RTMIN+{}", signal), "waybar"])
                    .status()
                    .ok();
            }
            Err(e) => {
                eprintln!("bar-updater: event error: {}", e);
            }
        }
    }
}

// ── WM detection (no IPC needed) ─────────────────────────────────────────────

fn detect_wm() -> &'static str {
    if env::var("SWAYSOCK").is_ok() {
        return "sway";
    }
    if env::var("I3SOCK").is_ok() {
        return "i3";
    }
    if Command::new("swaymsg")
        .arg("-t")
        .arg("get_version")
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return "sway";
    }
    "i3"
}

fn wm_msg_cmd() -> &'static str {
    if detect_wm() == "sway" { "swaymsg" } else { "i3-msg" }
}

fn display_id() -> String {
    if detect_wm() == "sway" {
        env::var("WAYLAND_DISPLAY")
            .unwrap_or_else(|_| "wayland-0".into())
            .replace(':', "")
    } else {
        env::var("DISPLAY")
            .unwrap_or_else(|_| ":0".into())
            .replace(':', "")
    }
}

fn socket_path() -> String {
    if let Ok(p) = env::var("I3_WORKSPACE_GROUPS_SOCKET") {
        return p;
    }
    let runtime_dir =
        env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    format!("{}/i3-workspace-groups-{}", runtime_dir, display_id())
}

// ── arg parsing helpers ───────────────────────────────────────────────────────

fn parse_group_context(args: &[String]) -> (GroupContext, Vec<String>) {
    let mut ctx = GroupContext::None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--group-active" => {
                ctx = GroupContext::Active;
            }
            "--group-focused" => {
                ctx = GroupContext::Focused;
            }
            "--group-name" => {
                i += 1;
                if i < args.len() {
                    ctx = GroupContext::Named(args[i].clone());
                }
            }
            other => rest.push(other.to_string()),
        }
        i += 1;
    }
    (ctx, rest)
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn get_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let pos = args.iter().position(|a| a == flag)?;
    args.get(pos + 1).map(|s| s.as_str())
}

// ── dispatch ──────────────────────────────────────────────────────────────────

fn dispatch(argv: &[String]) -> Result<String, String> {
    if argv.is_empty() {
        return Err("error: no command provided".to_string());
    }
    let cmd = argv[0].as_str();
    let rest: Vec<String> = argv[1..].to_vec();

    match cmd {
        "detect-wm" => return Ok(detect_wm().to_string()),
        "wm-msg" => {
            let wm_cmd = wm_msg_cmd();
            let status = Command::new(wm_cmd)
                .args(&rest)
                .status()
                .map_err(|e| format!("error: failed to run {}: {}", wm_cmd, e))?;
            process::exit(status.code().unwrap_or(1));
        }
        _ => {}
    }

    let mut conn = Connection::new()
        .map_err(|e| format!("error: could not connect to i3/sway: {}", e))?;

    let result = match cmd {
        "list-groups" => {
            let monitor_only = has_flag(&rest, "--focused-monitor-only");
            cmd_list_groups(&mut conn, monitor_only)
        }
        "list-workspaces" => {
            let default_fields = "global_number,group,static_name,dynamic_name,local_number,global_name,monitor,focused,window_icons";
            let fields_str =
                get_flag_value(&rest, "--fields").unwrap_or(default_fields);
            let fields: Vec<&str> = fields_str.split(',').collect();
            let focused_only = has_flag(&rest, "--focused-only");
            let focused_monitor_only = has_flag(&rest, "--focused-monitor-only");
            let (group_ctx, _) = parse_group_context(&rest);
            cmd_list_workspaces(
                &mut conn,
                &fields,
                focused_only,
                focused_monitor_only,
                &group_ctx,
            )
        }
        "workspace-number" => {
            let (group_ctx, positional) = parse_group_context(&rest);
            let no_back_forth = has_flag(&rest, "--no-auto-back-and-forth");
            let number: i64 = positional
                .iter()
                .find(|a| !a.starts_with('-'))
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "error: workspace-number requires a number".to_string())?;
            cmd_workspace_number(&mut conn, number, &group_ctx, !no_back_forth)
        }
        "workspace-next" => cmd_workspace_relative(&mut conn, 1),
        "workspace-prev" => cmd_workspace_relative(&mut conn, -1),
        "workspace-new" => {
            let (group_ctx, _) = parse_group_context(&rest);
            cmd_workspace_new(&mut conn, &group_ctx)
        }
        "move-to-number" => {
            let (group_ctx, positional) = parse_group_context(&rest);
            let no_back_forth = has_flag(&rest, "--no-auto-back-and-forth");
            let number: i64 = positional
                .iter()
                .find(|a| !a.starts_with('-'))
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "error: move-to-number requires a number".to_string())?;
            cmd_move_to_number(&mut conn, number, &group_ctx, no_back_forth)
        }
        "move-to-next" => cmd_move_relative(&mut conn, 1),
        "move-to-prev" => cmd_move_relative(&mut conn, -1),
        "move-to-new" => {
            let (group_ctx, _) = parse_group_context(&rest);
            cmd_move_to_new(&mut conn, &group_ctx)
        }
        "switch-active-group" => {
            let focused_monitor_only = has_flag(&rest, "--focused-monitor-only");
            let group = rest
                .iter()
                .find(|a| !a.starts_with('-'))
                .ok_or_else(|| {
                    "error: switch-active-group requires a group name".to_string()
                })?
                .clone();
            cmd_switch_active_group(&mut conn, &group, focused_monitor_only)
        }
        "rename-workspace" => {
            let name = get_flag_value(&rest, "--name").map(str::to_string);
            let number = get_flag_value(&rest, "--number")
                .and_then(|s| s.parse::<i64>().ok());
            let group = get_flag_value(&rest, "--group").map(str::to_string);
            cmd_rename_workspace(
                &mut conn,
                name.as_deref(),
                number,
                group.as_deref(),
            )
        }
        "assign-workspace-to-group" => {
            let group = rest
                .iter()
                .find(|a| !a.starts_with('-'))
                .ok_or_else(|| {
                    "error: assign-workspace-to-group requires a group name".to_string()
                })?
                .clone();
            cmd_assign_workspace_to_group(&mut conn, &group)
        }
        "waybar" => cmd_waybar(&mut conn),
        _ => return Err(format!("error: unknown command: {}", cmd)),
    };

    result.map_err(|e| format!("error: {}", e))
}

// ── server ────────────────────────────────────────────────────────────────────

fn run_server(socket_addr: &str) {
    use std::fs;

    if std::path::Path::new(socket_addr).exists() {
        match UnixStream::connect(socket_addr) {
            Ok(_) => {
                eprintln!("Another server is already running, exiting.");
                return;
            }
            Err(_) => {
                let _ = fs::remove_file(socket_addr);
            }
        }
    }

    let listener = UnixListener::bind(socket_addr).unwrap_or_else(|e| {
        eprintln!("error: could not bind socket {}: {}", socket_addr, e);
        process::exit(1);
    });

    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut data = Vec::new();
                if stream.read_to_end(&mut data).is_err() {
                    continue;
                }
                if data.len() > 10_000 {
                    continue;
                }
                let argv: Vec<String> =
                    match data
                        .split(|&b| b == b'\n')
                        .map(|s| String::from_utf8(s.to_vec()))
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                if argv.first().map(|s| s.as_str()) == Some("server") {
                    continue;
                }

                let response = match dispatch(&argv) {
                    Ok(s) => s,
                    Err(e) => e,
                };
                let _ = stream.write_all(response.as_bytes());
            }
            Err(e) => {
                eprintln!("error: accept failed: {}", e);
            }
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if let Some(first) = args.first() {
        match first.as_str() {
            "server" => {
                let addr = args
                    .iter()
                    .position(|a| a == "--server-addr")
                    .and_then(|i| args.get(i + 1))
                    .cloned()
                    .unwrap_or_else(socket_path);
                run_server(&addr);
                return;
            }
            "detect-wm" => {
                println!("{}", detect_wm());
                return;
            }
            "bar-updater" => {
                let signal = args
                    .iter()
                    .position(|a| a == "--waybar-signal")
                    .and_then(|i| args.get(i + 1))
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(8);
                run_bar_updater(signal);
                return;
            }
            "wm-msg" => {
                let cmd = wm_msg_cmd();
                let status = Command::new(cmd)
                    .args(&args[1..])
                    .status()
                    .unwrap_or_else(|e| {
                        eprintln!("error: failed to run {}: {}", cmd, e);
                        process::exit(1);
                    });
                process::exit(status.code().unwrap_or(1));
            }
            _ => {}
        }
    }

    // Client mode: forward to server via socket
    let payload = args.join("\n");
    let path = socket_path();

    let mut stream = UnixStream::connect(&path).unwrap_or_else(|e| {
        eprintln!("error: could not connect to server at {}: {}", path, e);
        eprintln!("Start the server with: swi3-sets server");
        process::exit(1);
    });

    stream.write_all(payload.as_bytes()).unwrap_or_else(|e| {
        eprintln!("error: failed to send command: {}", e);
        process::exit(1);
    });
    stream.shutdown(std::net::Shutdown::Write).ok();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap_or_else(|e| {
        eprintln!("error: failed to read response: {}", e);
        process::exit(1);
    });

    if !response.is_empty() {
        print!("{}", response);
        if !response.ends_with('\n') {
            println!();
        }
    }

    if response.starts_with("error:") {
        process::exit(1);
    }
}
