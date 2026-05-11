use std::collections::{BTreeMap, HashSet};
use std::env;
use std::io::Write;
use std::path::Path;
use std::process::{self, Command, Stdio};

use swayipc::{Connection, EventType, Fallible};

// ── workspace name format ────────────────────────────────────────────────────
// global_number:\u{200b}set\u{200b}:static_name\u{200b}:dynamic_name\u{200b}:local_number
const DELIM: char = '\u{200b}';
const MAX_SETS_PER_MONITOR: i64 = 1000;
const MAX_WORKSPACES_PER_SET: i64 = 100;

#[derive(Debug, Default, Clone)]
struct WsMeta {
    global_number: Option<i64>,
    set: Option<String>,
    static_name: Option<String>,
    dynamic_name: Option<String>,
    local_number: Option<i64>,
}

fn sanitize(s: &str) -> String {
    let s = s.replace(DELIM, "%");
    if s.starts_with(':') {
        s[1..].to_string()
    } else {
        s
    }
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
            set: Some(String::new()),
            ..Default::default()
        };
    }
    let global_number = if parts[0].is_empty() {
        None
    } else {
        strip_suffix_colon(parts[0]).parse::<i64>().ok()
    };
    let set = Some(if parts[1].is_empty() {
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
    WsMeta {
        global_number,
        set,
        static_name,
        dynamic_name,
        local_number,
    }
}

fn create_name(m: &WsMeta) -> String {
    let gn = m.global_number.expect("global_number required");
    let set = m.set.as_deref().unwrap_or("");
    let mut need_prefix = !set.is_empty();

    let static_part = match m.static_name.as_deref() {
        Some(s) if !s.is_empty() => {
            let v = if need_prefix {
                format!(":{}", s)
            } else {
                s.to_string()
            };
            need_prefix = true;
            v
        }
        _ => String::new(),
    };
    let dynamic_part = match m.dynamic_name.as_deref() {
        Some(s) if !s.is_empty() => {
            let v = if need_prefix {
                format!(":{}", s)
            } else {
                s.to_string()
            };
            need_prefix = true;
            v
        }
        _ => String::new(),
    };
    let local_part = match m.local_number {
        Some(n) => {
            if need_prefix {
                format!(":{}", n)
            } else {
                n.to_string()
            }
        }
        None => String::new(),
    };

    format!(
        "{}:{d}{}{d}{}{d}{}{d}{}",
        gn,
        set,
        static_part,
        dynamic_part,
        local_part,
        d = DELIM
    )
}

fn compute_global_number(monitor_index: i64, set_index: i64, local_number: i64) -> i64 {
    monitor_index * (MAX_SETS_PER_MONITOR * MAX_WORKSPACES_PER_SET)
        + set_index * MAX_WORKSPACES_PER_SET
        + local_number
}

fn global_number_to_set_index(global_number: i64) -> i64 {
    global_number % (MAX_SETS_PER_MONITOR * MAX_WORKSPACES_PER_SET) / MAX_WORKSPACES_PER_SET
}

fn global_number_to_local_number(global_number: i64) -> i64 {
    global_number % MAX_WORKSPACES_PER_SET
}

fn get_local_number(meta: &WsMeta) -> Option<i64> {
    meta.local_number
        .or_else(|| meta.global_number.map(global_number_to_local_number))
}

// ── IPC helpers ──────────────────────────────────────────────────────────────

fn get_sorted_outputs(conn: &mut Connection) -> Fallible<Vec<swayipc::Output>> {
    let mut outputs: Vec<swayipc::Output> = conn
        .get_outputs()?
        .into_iter()
        .filter(|o| o.active)
        .collect();
    outputs.sort_by_key(|o| (o.rect.y, o.rect.x));
    Ok(outputs)
}

fn get_monitor_index(conn: &mut Connection, monitor_name: &str) -> Fallible<i64> {
    let outputs = get_sorted_outputs(conn)?;
    Ok(outputs
        .iter()
        .position(|o| o.name == monitor_name)
        .unwrap_or(0) as i64)
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

/// Ordered map: set → Vec<Workspace> in encounter order (preserves i3 order).
fn set_to_workspaces_ordered(
    workspaces: &[swayipc::Workspace],
) -> Vec<(String, Vec<swayipc::Workspace>)> {
    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, Vec<swayipc::Workspace>> = BTreeMap::new();
    for ws in workspaces {
        let set = parse_name(&ws.name).set.unwrap_or_default();
        if !map.contains_key(&set) {
            order.push(set.clone());
        }
        map.entry(set).or_default().push(ws.clone());
    }
    order
        .into_iter()
        .map(|g| {
            let mut v = map.remove(&g).unwrap();
            v.sort_by_key(|ws| get_local_number(&parse_name(&ws.name)).unwrap_or(i64::MAX));
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

// ── set index ───────────────────────────────────────────────────────────────

fn get_set_index(target_group: &str, groups: &[(String, Vec<swayipc::Workspace>)]) -> i64 {
    let mut set_to_index: BTreeMap<String, i64> = BTreeMap::new();
    for (set, workspaces) in groups {
        for ws in workspaces {
            if let Some(gn) = parse_name(&ws.name).global_number {
                set_to_index
                    .entry(set.clone())
                    .or_insert_with(|| global_number_to_set_index(gn));
                break;
            }
        }
    }
    if let Some(&idx) = set_to_index.get(target_group) {
        return idx;
    }
    set_to_index
        .values()
        .copied()
        .max()
        .map(|m| m + 1)
        .unwrap_or(0)
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
    let groups = set_to_workspaces_ordered(&all);
    let set_workspaces: Vec<swayipc::Workspace> = groups
        .into_iter()
        .filter(|(s, _)| s == target_group)
        .flat_map(|(_, ws)| ws)
        .collect();
    let used = get_used_local_numbers(&set_workspaces);
    Ok(get_lowest_free_local_numbers(1, &used)[0])
}

// ── organize workspace sets (renumber) ──────────────────────────────────────

fn organize_workspace_sets(
    conn: &mut Connection,
    monitor_name: &str,
    ordered_sets: &[(String, Vec<swayipc::Workspace>)],
    all_workspaces: &[swayipc::Workspace],
) -> Fallible<()> {
    let monitor_index = get_monitor_index(conn, monitor_name)?;
    let all_sets = set_to_workspaces_ordered(all_workspaces);

    for (set_index, (set, workspaces)) in ordered_sets.iter().enumerate() {
        let monitor_ws_names: HashSet<String> =
            workspaces.iter().map(|ws| ws.name.clone()).collect();

        // Local numbers used by this set on OTHER monitors
        let other_set_ws: Vec<swayipc::Workspace> = all_sets
            .iter()
            .filter(|(s, _)| s == set)
            .flat_map(|(_, ws)| {
                ws.iter()
                    .filter(|w| !monitor_ws_names.contains(&w.name))
                    .cloned()
            })
            .collect();
        let used_in_others = get_used_local_numbers(&other_set_ws);

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
            new_meta.set = Some(set.clone());
            new_meta.local_number = Some(ln);
            new_meta.global_number =
                Some(compute_global_number(monitor_index, set_index as i64, ln));
            new_meta.dynamic_name = Some(String::new());
            let new_name = create_name(&new_meta);
            rename_workspace(conn, &ws.name, &new_name)?;
        }
    }
    Ok(())
}

// ── set context ─────────────────────────────────────────────────────────────

enum SetContext {
    Active,
    Focused,
    Named(String),
    None,
}

fn resolve_set(conn: &mut Connection, ctx: &SetContext) -> Fallible<String> {
    match ctx {
        SetContext::Named(name) => Ok(name.clone()),
        SetContext::Focused => {
            let ws = get_focused_workspace(conn)?;
            Ok(parse_name(&ws.name).set.unwrap_or_default())
        }
        SetContext::Active | SetContext::None => {
            let monitor = get_focused_monitor_name(conn)?;
            let ws = get_monitor_workspaces(conn, &monitor)?;
            let groups = set_to_workspaces_ordered(&ws);
            Ok(groups
                .into_iter()
                .next()
                .map(|(s, _)| s)
                .unwrap_or_default())
        }
    }
}

// ── workspace-by-local-number ─────────────────────────────────────────────────

fn create_workspace_name_for(
    conn: &mut Connection,
    set: &str,
    local_number: i64,
) -> Fallible<String> {
    let monitor = get_focused_monitor_name(conn)?;
    let monitor_index = get_monitor_index(conn, &monitor)?;
    let monitor_ws = get_monitor_workspaces(conn, &monitor)?;
    let groups = set_to_workspaces_ordered(&monitor_ws);
    let set_index = get_set_index(set, &groups);
    let gn = compute_global_number(monitor_index, set_index, local_number);
    let meta = WsMeta {
        global_number: Some(gn),
        set: Some(set.to_string()),
        local_number: Some(local_number),
        ..Default::default()
    };
    Ok(create_name(&meta))
}

fn get_workspace_by_local_number(
    conn: &mut Connection,
    set: &str,
    local_number: i64,
) -> Fallible<(String, bool)> {
    let all = get_all_workspaces(conn)?;
    for ws in &all {
        let meta = parse_name(&ws.name);
        if meta.set.as_deref() == Some(set) && get_local_number(&meta) == Some(local_number) {
            return Ok((ws.name.clone(), true));
        }
    }
    Ok((create_workspace_name_for(conn, set, local_number)?, false))
}

fn cmd_switch_rewind(conn: &mut Connection) -> Fallible<String> {
    send_i3_command(conn, "workspace back_and_forth")?;
    Ok(String::new())
}

// ── command handlers ──────────────────────────────────────────────────────────

fn cmd_list_groups(conn: &mut Connection, focused_monitor_only: bool) -> Fallible<String> {
    let workspaces = if focused_monitor_only {
        let monitor = get_focused_monitor_name(conn)?;
        get_monitor_workspaces(conn, &monitor)?
    } else {
        get_all_workspaces(conn)?
    };
    let groups = set_to_workspaces_ordered(&workspaces);
    let names: Vec<String> = groups.into_iter().map(|(s, _)| s).collect();
    Ok(names.join("\n") + "\n")
}

fn cmd_list_workspaces(
    conn: &mut Connection,
    fields: &[&str],
    focused_only: bool,
    focused_monitor_only: bool,
    set_ctx: &SetContext,
) -> Fallible<String> {
    let workspaces = if focused_monitor_only {
        let monitor = get_focused_monitor_name(conn)?;
        get_monitor_workspaces(conn, &monitor)?
    } else {
        get_all_workspaces(conn)?
    };

    let target_workspaces: Vec<swayipc::Workspace> = match set_ctx {
        SetContext::None => workspaces,
        _ => {
            let target_group = resolve_set(conn, set_ctx)?;
            let groups = set_to_workspaces_ordered(&workspaces);
            groups
                .into_iter()
                .filter(|(s, _)| *s == target_group)
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
                    "set" => meta.set.clone().unwrap_or_default(),
                    "static_name" => meta.static_name.clone().unwrap_or_default(),
                    "dynamic_name" => meta.dynamic_name.clone().unwrap_or_default(),
                    "local_number" => get_local_number(&meta)
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    "global_name" => ws.name.clone(),
                    "monitor" => ws.output.clone(),
                    "focused" => {
                        if ws.focused {
                            "1".to_string()
                        } else {
                            "0".to_string()
                        }
                    }
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
    set_ctx: &SetContext,
    auto_back_and_forth: bool,
) -> Fallible<String> {
    let set = resolve_set(conn, set_ctx)?;
    let (name, _) = get_workspace_by_local_number(conn, &set, local_number)?;
    focus_workspace(conn, &name, auto_back_and_forth)?;
    Ok(String::new())
}

fn cmd_workspace_relative(conn: &mut Connection, offset: i64) -> Fallible<String> {
    let focused = get_focused_workspace(conn)?;
    let focused_set = parse_name(&focused.name).set.unwrap_or_default();
    let focused_output = focused.output.clone();
    let all = get_all_workspaces(conn)?;
    let groups = set_to_workspaces_ordered(&all);
    let set_ws: Vec<swayipc::Workspace> = groups
        .into_iter()
        .filter(|(s, _)| *s == focused_set)
        .flat_map(|(_, ws)| ws)
        .filter(|ws| ws.output == focused_output)
        .collect();
    if set_ws.is_empty() {
        return Ok(String::new());
    }
    let current = set_ws
        .iter()
        .position(|ws| ws.name == focused.name)
        .unwrap_or(0);
    let next = ((current as i64 + offset).rem_euclid(set_ws.len() as i64)) as usize;
    focus_workspace(conn, &set_ws[next].name, false)?;
    Ok(String::new())
}

fn cmd_workspace_global_relative(conn: &mut Connection, offset: i64) -> Fallible<String> {
    let focused = get_focused_workspace(conn)?;
    let focused_output = focused.output.clone();
    let all = get_all_workspaces(conn)?;
    let mut groups = set_to_workspaces_ordered(&all);
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let flat: Vec<(String, swayipc::Workspace)> = groups
        .into_iter()
        .flat_map(|(g, ws)| ws.into_iter().map(move |w| (g.clone(), w)))
        .filter(|(_, ws)| ws.output == focused_output)
        .collect();
    if flat.is_empty() {
        return Ok(String::new());
    }
    let current = flat
        .iter()
        .position(|(_, ws)| ws.name == focused.name)
        .unwrap_or(0);
    let next = ((current as i64 + offset).rem_euclid(flat.len() as i64)) as usize;
    let (_target_group, target_ws) = &flat[next];
    focus_workspace(conn, &target_ws.name, false)?;
    Ok(String::new())
}

fn cmd_workspace_new(conn: &mut Connection, set_ctx: &SetContext) -> Fallible<String> {
    let set = resolve_set(conn, set_ctx)?;
    let ln = find_free_local_number(conn, &set)?;
    cmd_workspace_number(conn, ln, set_ctx, true)
}

fn cmd_move_to_number(
    conn: &mut Connection,
    local_number: i64,
    set_ctx: &SetContext,
    no_auto_back_and_forth: bool,
) -> Fallible<String> {
    let set = resolve_set(conn, set_ctx)?;
    let (name, _) = get_workspace_by_local_number(conn, &set, local_number)?;
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
    let focused_set = parse_name(&focused.name).set.unwrap_or_default();
    let all = get_all_workspaces(conn)?;
    let groups = set_to_workspaces_ordered(&all);
    let set_ws: Vec<swayipc::Workspace> = groups
        .into_iter()
        .filter(|(s, _)| *s == focused_set)
        .flat_map(|(_, ws)| ws)
        .collect();
    if set_ws.is_empty() {
        return Ok(String::new());
    }
    let current = set_ws
        .iter()
        .position(|ws| ws.name == focused.name)
        .unwrap_or(0);
    let next = ((current as i64 + offset).rem_euclid(set_ws.len() as i64)) as usize;
    send_i3_command(
        conn,
        &format!(
            "move container to workspace \"{}\"",
            set_ws[next].name.replace('"', "\\\"")
        ),
    )?;
    Ok(String::new())
}

fn cmd_move_to_new(conn: &mut Connection, set_ctx: &SetContext) -> Fallible<String> {
    let set = resolve_set(conn, set_ctx)?;
    let ln = find_free_local_number(conn, &set)?;
    cmd_move_to_number(conn, ln, set_ctx, false)
}

fn cmd_switch_active_group(
    conn: &mut Connection,
    target_group: &str,
    focused_monitor_only: bool,
) -> Fallible<String> {
    let focused_monitor = get_focused_monitor_name(conn)?;
    let monitor_to_ws = get_monitor_to_workspaces(conn)?;

    for (monitor, workspaces) in &monitor_to_ws {
        let groups = set_to_workspaces_ordered(workspaces);
        let set_exists = groups.iter().any(|(s, _)| s == target_group);
        if monitor != &focused_monitor && (focused_monitor_only || !set_exists) {
            continue;
        }

        let mut reordered: Vec<(String, Vec<swayipc::Workspace>)> = Vec::new();
        let target_ws: Vec<swayipc::Workspace> = groups
            .iter()
            .filter(|(s, _)| s == target_group)
            .flat_map(|(_, ws)| ws.clone())
            .collect();
        reordered.push((target_group.to_string(), target_ws));
        for (s, ws) in &groups {
            if s != target_group {
                reordered.push((s.clone(), ws.clone()));
            }
        }

        let all_ws = get_all_workspaces(conn)?;
        organize_workspace_sets(conn, monitor, &reordered, &all_ws)?;
    }

    // Switch focus if needed — re-fetch after renames
    let focused_ws = get_focused_workspace(conn)?;
    let focused_set = parse_name(&focused_ws.name).set.unwrap_or_default();
    if focused_set == target_group {
        focus_workspace(conn, &focused_ws.name, false)?;
        return Ok(String::new());
    }

    let monitor_ws = get_monitor_workspaces(conn, &focused_monitor)?;
    let groups = set_to_workspaces_ordered(&monitor_ws);
    let workspace_name = groups
        .iter()
        .find(|(s, _)| s == target_group)
        .and_then(|(_, ws)| ws.first().map(|w| w.name.clone()));

    let name = if let Some(n) = workspace_name {
        n
    } else {
        let monitor_index = get_monitor_index(conn, &focused_monitor)?;
        let groups2 = set_to_workspaces_ordered(&monitor_ws);
        let set_index = get_set_index(target_group, &groups2);
        let ln = find_free_local_number(conn, target_group)?;
        let gn = compute_global_number(monitor_index, set_index, ln);
        create_name(&WsMeta {
            global_number: Some(gn),
            set: Some(target_group.to_string()),
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
    set: Option<&str>,
) -> Fallible<String> {
    let focused = get_focused_workspace(conn)?;
    let mut meta = parse_name(&focused.name);
    if let Some(n) = new_name {
        meta.static_name = Some(n.to_string());
    }
    if let Some(n) = number {
        meta.local_number = Some(n);
    }
    if let Some(s) = set {
        meta.set = Some(s.to_string());
    }

    let s = meta.set.clone().unwrap_or_default();
    let ln = meta.local_number.unwrap_or(1);

    // Check for conflict with another workspace
    let all = get_all_workspaces(conn)?;
    let groups = set_to_workspaces_ordered(&all);
    let conflict = groups
        .iter()
        .filter(|(grp, _)| grp == &s)
        .flat_map(|(_, ws)| ws)
        .find(|ws| ws.name != focused.name && get_local_number(&parse_name(&ws.name)) == Some(ln))
        .is_some();

    if conflict {
        let used: HashSet<i64> = groups
            .iter()
            .filter(|(grp, _)| grp == &s)
            .flat_map(|(_, ws)| ws)
            .filter_map(|ws| parse_name(&ws.name).local_number)
            .collect();
        meta.local_number = Some(get_lowest_free_local_numbers(1, &used)[0]);
    }

    let monitor = get_focused_monitor_name(conn)?;
    let monitor_index = get_monitor_index(conn, &monitor)?;
    let monitor_ws = get_monitor_workspaces(conn, &monitor)?;
    let groups2 = set_to_workspaces_ordered(&monitor_ws);
    let set_index = get_set_index(&s, &groups2);
    let local_number = meta.local_number.unwrap_or(1);
    meta.global_number = Some(compute_global_number(
        monitor_index,
        set_index,
        local_number,
    ));
    meta.dynamic_name = Some(String::new());

    let new_ws_name = create_name(&meta);
    rename_workspace(conn, &focused.name, &new_ws_name)?;
    Ok(String::new())
}

fn cmd_assign_workspace_to_group(conn: &mut Connection, group: &str) -> Fallible<String> {
    cmd_rename_workspace(conn, None, None, Some(group))
}

/// Initialize a fresh i3/sway session: place one workspace per monitor in
/// left-to-right order (1 on the leftmost, 2 on the next, ...), all in the
/// `<default>` set, and focus the leftmost monitor.
///
/// Idempotent: if any workspace already has a set assigned or an encoded
/// global_number, the session is considered non-fresh and this command is a
/// no-op. That way it's safe to `exec` unconditionally from the WM config.
fn cmd_init_session(conn: &mut Connection) -> Fallible<String> {
    let workspaces = get_all_workspaces(conn)?;

    // Fresh = every existing workspace name is a bare positive integer, i.e.
    // the default i3/sway naming with no set and no encoded prefix.
    let is_fresh = !workspaces.is_empty()
        && workspaces
            .iter()
            .all(|ws| !ws.name.is_empty() && ws.name.chars().all(|c| c.is_ascii_digit()));
    if !is_fresh {
        return Ok(String::new());
    }

    let outputs = get_sorted_outputs(conn)?;
    if outputs.is_empty() {
        return Ok(String::new());
    }

    // Step 1: ensure a plain numbered workspace exists on each monitor, in
    // left-to-right order. `focus output` moves focus to the target monitor,
    // then `workspace number N` creates/focuses workspace N there.
    for (i, out) in outputs.iter().enumerate() {
        let n = (i + 1) as i64;
        send_i3_command(conn, &format!("focus output \"{}\"", out.name))?;
        send_i3_command(conn, &format!("workspace number {}", n))?;
    }

    // Step 2: rename each of those workspaces into the encoded form used by
    // swi3-groups, placing them in the `<default>` (empty) set.
    let workspaces = get_all_workspaces(conn)?;
    for (i, out) in outputs.iter().enumerate() {
        let n = (i + 1) as i64;
        let expected = n.to_string();
        let Some(ws) = workspaces
            .iter()
            .find(|w| w.output == out.name && w.name == expected)
        else {
            continue;
        };
        let gn = compute_global_number(i as i64, 0, n);
        let new_meta = WsMeta {
            global_number: Some(gn),
            set: Some(String::new()),
            local_number: Some(n),
            ..Default::default()
        };
        let new_name = create_name(&new_meta);
        if new_name != ws.name {
            rename_workspace(conn, &ws.name, &new_name)?;
        }
    }

    // Step 3: leave focus on the leftmost monitor (workspace 1).
    send_i3_command(conn, &format!("focus output \"{}\"", outputs[0].name))?;

    Ok(String::new())
}

fn cmd_waybar(conn: &mut Connection) -> Fallible<String> {
    let workspaces = get_all_workspaces(conn)?;

    let current_set = workspaces
        .iter()
        .find(|ws| ws.focused)
        .map(|ws| parse_name(&ws.name).set.unwrap_or_default())
        .unwrap_or_default();

    let groups = set_to_workspaces_ordered(&workspaces);

    let mut text_parts: Vec<String> = Vec::new();
    let mut tooltip_lines: Vec<String> = Vec::new();

    for (set, set_ws) in &groups {
        let display = if set.is_empty() {
            "default"
        } else {
            set.as_str()
        };

        if *set == current_set {
            // Active group: one text entry per workspace
            for ws in set_ws {
                let meta = parse_name(&ws.name);
                let local_num = get_local_number(&meta).unwrap_or(0);
                text_parts.push(format!("{}:{}", display, local_num));
            }
        } else {
            // Inactive group: just the group name
            text_parts.push(display.to_string());
        }

        for ws in set_ws {
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
    let class = if current_set.is_empty() {
        "default".to_string()
    } else {
        current_set
    };

    Ok(format!(
        r#"{{"text": "{}", "tooltip": "{}", "class": "{}"}}"#,
        text, tooltip, class
    ))
}

fn run_bar_updater(signal: u32) {
    let subs = [EventType::Workspace];

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

// ── menu helpers (rofi/wofi/fuzzel/dmenu) ───────────────────────────────────

/// True if rofi's pidfile points at a live process. Two filesystem ops total
/// — avoids scanning /proc on every menu launch.
fn rofi_lock_held() -> bool {
    let Ok(runtime) = env::var("XDG_RUNTIME_DIR") else { return false };
    let pidfile = Path::new(&runtime).join("rofi.pid");
    let Ok(content) = std::fs::read_to_string(&pidfile) else { return false };
    let Ok(pid) = content.trim().parse::<u32>() else { return false };
    Path::new("/proc").join(pid.to_string()).join("comm").exists()
}

fn command_exists(cmd: &str) -> bool {
    if cmd.contains('/') {
        return Path::new(cmd).is_file();
    }
    if let Ok(path) = env::var("PATH") {
        for dir in path.split(':') {
            if !dir.is_empty() && Path::new(dir).join(cmd).exists() {
                return true;
            }
        }
    }
    false
}

struct MenuCmd {
    cmd: String,
    args: Vec<String>,
}

fn detect_menu_backend() -> Option<String> {
    if let Ok(m) = env::var("SWI3SETS_MENU") {
        if !m.trim().is_empty() {
            return Some(m);
        }
    }
    for cmd in &["rofi", "wofi", "fuzzel", "dmenu"] {
        if command_exists(cmd) {
            return Some((*cmd).to_string());
        }
    }
    None
}

fn build_menu(
    prompt: &str,
    mesg: &str,
    theme: &str,
    focused_output: Option<&str>,
) -> Option<MenuCmd> {
    let backend = detect_menu_backend()?;
    let head = backend.split_whitespace().next().unwrap_or("").to_string();
    let mut args: Vec<String> = Vec::new();
    match head.as_str() {
        "rofi" => {
            args.extend([
                "-dmenu".to_string(),
                "-p".to_string(),
                prompt.to_string(),
                "-kb-cancel".to_string(),
                "Escape".to_string(),
            ]);
            if let Some(out) = focused_output {
                if !out.is_empty() {
                    args.push("-monitor".to_string());
                    args.push(out.to_string());
                }
            }
            if !theme.is_empty() {
                args.push("-theme-str".to_string());
                args.push(theme.to_string());
            }
            if !mesg.is_empty() {
                args.push("-mesg".to_string());
                args.push(mesg.to_string());
            }
            Some(MenuCmd { cmd: "rofi".to_string(), args })
        }
        "wofi" => {
            args.extend([
                "--dmenu".to_string(),
                "--prompt".to_string(),
                prompt.to_string(),
            ]);
            Some(MenuCmd { cmd: "wofi".to_string(), args })
        }
        "fuzzel" => {
            args.extend([
                "--dmenu".to_string(),
                "--prompt".to_string(),
                prompt.to_string(),
            ]);
            Some(MenuCmd { cmd: "fuzzel".to_string(), args })
        }
        "dmenu" => {
            args.extend(["-p".to_string(), prompt.to_string()]);
            Some(MenuCmd { cmd: "dmenu".to_string(), args })
        }
        _ => {
            // Custom user-supplied command, possibly with embedded args.
            let mut parts = backend.split_whitespace().map(String::from);
            let cmd = parts.next()?;
            let args: Vec<String> = parts.collect();
            Some(MenuCmd { cmd, args })
        }
    }
}

fn run_menu(menu: &MenuCmd, input: &str) -> Result<Option<String>, String> {
    // rofi uses an exclusive pidfile and refuses to start while another
    // instance is alive. Only pay the kill+settle cost when one is actually
    // running; the /proc scan is sub-millisecond and avoids forking pkill
    // on every keypress.
    let basename = Path::new(&menu.cmd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(menu.cmd.as_str());
    if basename == "rofi" && rofi_lock_held() {
        let _ = Command::new("pkill").args(["-x", "rofi"]).status();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    let mut child = Command::new(&menu.cmd)
        .args(&menu.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn menu '{}': {}", menu.cmd, e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|e| format!("failed to write to menu stdin: {}", e))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("menu wait failed: {}", e))?;
    if !output.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string();
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

fn focused_output_for_menu(conn: &mut Connection) -> Option<String> {
    get_focused_monitor_name(conn).ok()
}

/// Format rows into a left-aligned column table with the given separator.
fn format_table(rows: &[Vec<String>], separator: &str) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; ncols];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    rows.iter()
        .map(|r| {
            let parts: Vec<String> = r
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let w = widths[i];
                    let pad = w.saturating_sub(c.chars().count());
                    format!("{}{}", c, " ".repeat(pad))
                })
                .collect();
            parts.join(separator).trim_end().to_string()
        })
        .collect()
}

// ── high-level commands previously in shell scripts ─────────────────────────

const DEFAULT_SET_ITEM: &str = "<default>";

fn list_sets(conn: &mut Connection, focused_monitor_only: bool) -> Fallible<Vec<String>> {
    let workspaces = if focused_monitor_only {
        let monitor = get_focused_monitor_name(conn)?;
        get_monitor_workspaces(conn, &monitor)?
    } else {
        get_all_workspaces(conn)?
    };
    Ok(set_to_workspaces_ordered(&workspaces)
        .into_iter()
        .map(|(s, _)| s)
        .collect())
}

fn cmd_select_set(conn: &mut Connection, mesg: &str) -> Result<Option<String>, String> {
    let workspaces = get_all_workspaces(conn).map_err(|e| e.to_string())?;
    let focused_out = workspaces
        .iter()
        .find(|ws| ws.focused)
        .map(|ws| ws.output.clone());
    let groups = set_to_workspaces_ordered(&workspaces);
    let mut input = String::new();

    let has_default = groups.iter().any(|(s, _)| s.is_empty());
    if !has_default {
        input.push_str(DEFAULT_SET_ITEM);
        input.push('\n');
    }
    for (s, _) in groups {
        if s.is_empty() {
            input.push_str(DEFAULT_SET_ITEM);
        } else {
            input.push_str(&s);
        }
        input.push('\n');
    }

    let menu = build_menu(
        "Workspace Group",
        mesg,
        "window {width: 60ch;} listview {lines: 10;}",
        focused_out.as_deref(),
    )
    .ok_or_else(|| "no menu launcher found".to_string())?;
    
    let chosen = run_menu(&menu, &input)?;
    Ok(chosen.map(|c| if c == DEFAULT_SET_ITEM { String::new() } else { c }))
}

fn cmd_switch_set(conn: &mut Connection) -> Result<String, String> {
    let mesg = "<span alpha=\"50%\" size=\"smaller\"><b>Select a workspace group to activate.</b>\n\
<i>You can create a new group by typing a new name.\nNote that the default group is shown as &lt;default>.</i></span>";
    let Some(set) = cmd_select_set(conn, mesg)? else {
        return Ok(String::new());
    };
    cmd_switch_active_group(conn, &set, false).map_err(|e| e.to_string())
}

fn cmd_assign_menu(conn: &mut Connection) -> Result<String, String> {
    let mesg = "<span alpha=\"50%\" size=\"smaller\"><b>Select a group to assign to the focused workspace.</b>\n\
<i>You can assign it to a new group by typing a new name.\nNote that the default group is shown as &lt;default>.</i></span>";
    let Some(set) = cmd_select_set(conn, mesg)? else {
        return Ok(String::new());
    };
    cmd_assign_workspace_to_group(conn, &set).map_err(|e| e.to_string())
}

fn cmd_assign_switch_menu(conn: &mut Connection) -> Result<String, String> {
    let mesg = "<span alpha=\"50%\" size=\"smaller\"><b>Select a group to assign the focused workspace to and switch to.</b>\n\
<i>You can assign it to a new group by typing a new name.\nNote that the default group is shown as &lt;default>.</i></span>";
    let Some(set) = cmd_select_set(conn, mesg)? else {
        return Ok(String::new());
    };
    cmd_assign_workspace_to_group(conn, &set).map_err(|e| e.to_string())?;
    cmd_switch_active_group(conn, &set, false).map_err(|e| e.to_string())
}

fn cmd_focus_menu(conn: &mut Connection) -> Result<String, String> {
    let mesg = "<span alpha=\"50%\" size=\"smaller\"><b>Select a workspace to focus on</b>\n\
<i>You can focus on a new (non existing) workspace by using the format \"set:number\", for example \"work:2\"</i></span>";
    let workspaces = get_all_workspaces(conn).map_err(|e| e.to_string())?;
    let rows: Vec<Vec<String>> = workspaces
        .iter()
        .map(|ws| {
            let m = parse_name(&ws.name);
            vec![
                m.set.clone().unwrap_or_default(),
                get_local_number(&m).map(|n| n.to_string()).unwrap_or_default(),
                String::new(),
                m.static_name.clone().unwrap_or_default(),
            ]
        })
        .collect();
    let displayed = format_table(&rows, "    ");
    let global_names: Vec<String> = workspaces.iter().map(|w| w.name.clone()).collect();
    let focused_out = focused_output_for_menu(conn);
    let menu = build_menu(
        "Workspace",
        mesg,
        "window {width: 60ch;}",
        focused_out.as_deref(),
    )
    .ok_or_else(|| "no menu launcher found".to_string())?;
    let input = displayed.join("\n");
    let Some(selected) = run_menu(&menu, &input)? else {
        return Ok(String::new());
    };
    if let Some(idx) = displayed.iter().position(|line| *line == selected) {
        let target = &global_names[idx];
        focus_workspace(conn, target, false).map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    // Fall back to "set:local_number" form.
    let (set, num) = parse_set_local(&selected)?;
    cmd_workspace_number(conn, num, &SetContext::Named(set), true).map_err(|e| e.to_string())
}

fn cmd_move_menu(conn: &mut Connection) -> Result<String, String> {
    let mesg = "<span alpha=\"50%\" size=\"smaller\"><b>Select a workspace to move the focused container into</b>\n\
<i>You can select a new (non existing) workspace by using the format \"set:number\", for example \"work:2\".\nDisplayed columns: set, number, window icons, name.</i></span>";
    let workspaces = get_all_workspaces(conn).map_err(|e| e.to_string())?;
    let rows: Vec<Vec<String>> = workspaces
        .iter()
        .map(|ws| {
            let m = parse_name(&ws.name);
            vec![
                m.set.clone().unwrap_or_default(),
                get_local_number(&m).map(|n| n.to_string()).unwrap_or_default(),
                String::new(),
                m.static_name.clone().unwrap_or_default(),
            ]
        })
        .collect();
    let displayed = format_table(&rows, "    ");
    let global_names: Vec<String> = workspaces.iter().map(|w| w.name.clone()).collect();
    let focused_out = focused_output_for_menu(conn);
    let menu = build_menu(
        "Workspace",
        mesg,
        "window {width: 60ch;}",
        focused_out.as_deref(),
    )
    .ok_or_else(|| "no menu launcher found".to_string())?;
    let input = displayed.join("\n");
    let Some(selected) = run_menu(&menu, &input)? else {
        return Ok(String::new());
    };
    if let Some(idx) = displayed.iter().position(|line| *line == selected) {
        let target = &global_names[idx];
        send_i3_command(
            conn,
            &format!(
                "move container to workspace \"{}\"",
                target.replace('"', "\\\"")
            ),
        )
        .map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    let (set, num) = parse_set_local(&selected)?;
    cmd_move_to_number(conn, num, &SetContext::Named(set), false).map_err(|e| e.to_string())
}

fn parse_set_local(s: &str) -> Result<(String, i64), String> {
    let mut it = s.splitn(2, ':');
    let set = it.next().unwrap_or("").to_string();
    let num_str = it
        .next()
        .ok_or_else(|| format!("invalid 'set:number' input: {}", s))?;
    let num: i64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in '{}'", s))?;
    Ok((set, num))
}

fn cmd_rename_menu(conn: &mut Connection) -> Result<String, String> {
    let focused = get_focused_workspace(conn).map_err(|e| e.to_string())?;
    let current_name = parse_name(&focused.name)
        .static_name
        .unwrap_or_default();
    let mesg = format!(
        "<span alpha=\"50%\" size=\"smaller\"><b>Enter a new name for workspace \"{}\"</b>\n\
<i>By default only the name is changed, but you can also use colons to change the number or set, In addition, you can use an hyphen (\"-\") to reset a property.\n\
Examples:  foo  |  foo:2  |  -:2  |  :2  |  bar:foo:2  |  bar::2</i></span>",
        current_name
    );
    let focused_out = focused_output_for_menu(conn);
    let menu = build_menu(
        "Rename",
        &mesg,
        "window {width: 60ch;} listview {lines: 0;}",
        focused_out.as_deref(),
    )
    .ok_or_else(|| "no menu launcher found".to_string())?;
    let Some(pattern) = run_menu(&menu, "")? else {
        return Ok(String::new());
    };
    apply_rename_pattern(conn, &pattern)
}

fn apply_rename_pattern(conn: &mut Connection, pattern: &str) -> Result<String, String> {
    let parts: Vec<&str> = pattern.split(':').collect();
    let mut name: Option<String> = None;
    let mut number: Option<i64> = None;
    let mut set: Option<String> = None;

    let resolve = |s: &str| -> Option<String> {
        if s == "-" {
            Some(String::new())
        } else if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    };
    let resolve_num = |s: &str| -> Result<Option<i64>, String> {
        if s == "-" || s.is_empty() {
            Ok(None)
        } else {
            s.parse::<i64>()
                .map(Some)
                .map_err(|_| format!("invalid number in rename pattern: {}", s))
        }
    };

    match parts.len() {
        0 => {}
        1 => {
            name = resolve(parts[0]);
        }
        2 => {
            name = resolve(parts[0]);
            number = resolve_num(parts[1])?;
        }
        3 => {
            set = resolve(parts[0]);
            name = resolve(parts[1]);
            number = resolve_num(parts[2])?;
        }
        _ => return Err("name pattern cannot contain more than 3 colons".to_string()),
    }
    cmd_rename_workspace(conn, name.as_deref(), number, set.as_deref())
        .map_err(|e| e.to_string())
}

// ── polybar text formatter ──────────────────────────────────────────────────

#[derive(Default, Clone)]
struct PolybarOpts {
    color_external_monitor: String,
    color_current_ws: String,
    shorthand: bool,
}

fn parse_polybar_opts(args: &[String]) -> PolybarOpts {
    let mut o = PolybarOpts::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-c" => {
                i += 1;
                if i < args.len() {
                    o.color_current_ws = args[i].clone();
                }
            }
            "-e" => {
                i += 1;
                if i < args.len() {
                    o.color_external_monitor = args[i].clone();
                }
            }
            "-s" => o.shorthand = true,
            _ => {}
        }
        i += 1;
    }
    o
}

fn cmd_polybar(conn: &mut Connection, opts: &PolybarOpts) -> Fallible<String> {
    let workspaces = get_all_workspaces(conn)?;

    let mut globals: Vec<i64> = Vec::new();
    let mut sets: Vec<String> = Vec::new();
    let mut locals: Vec<i64> = Vec::new();
    let mut focused: Vec<bool> = Vec::new();
    for ws in &workspaces {
        let m = parse_name(&ws.name);
        globals.push(m.global_number.unwrap_or(0));
        sets.push(m.set.clone().unwrap_or_default());
        locals.push(get_local_number(&m).unwrap_or(0));
        focused.push(ws.focused);
    }

    let (current_set, current_local) = focused
        .iter()
        .position(|f| *f)
        .map(|i| (sets[i].clone(), locals[i]))
        .unwrap_or((String::new(), 0));

    let mut seen_sets: Vec<String> = Vec::new();
    for s in &sets {
        if !seen_sets.contains(s) {
            seen_sets.push(s.clone());
        }
    }

    let ows = |ws_num: i64, set: &str| -> String {
        let s = if set == "default" { "" } else { set };
        format!("swi3-groups workspace-number {} --group-name \"{}\"", ws_num, s)
    };
    let ch_st = |set: &str| -> String {
        let s = if set == "default" { "" } else { set };
        format!("swi3-groups switch-active-group \"{}\"", s)
    };
    let txt_item = |display: &str, set: &str, ws_num: i64, global_num: i64| -> String {
        let ws_screen = global_num / 100000;
        if ws_screen == 0 || opts.color_external_monitor.is_empty() {
            format!("%{{A1:{}:}}{}%{{A}}", ows(ws_num, set), display)
        } else {
            // Approximate: color the digit run inside `display`.
            let mut colored = String::new();
            let mut chars = display.chars().peekable();
            let mut wrote = false;
            while let Some(c) = chars.next() {
                if !wrote && c.is_ascii_digit() {
                    colored.push_str(&format!("%{{F{}}}", opts.color_external_monitor));
                    colored.push(c);
                    while let Some(&nc) = chars.peek() {
                        if nc.is_ascii_digit() {
                            colored.push(nc);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    colored.push_str("%{F-}");
                    wrote = true;
                } else {
                    colored.push(c);
                }
            }
            format!("%{{A1:{}:}}{}%{{A}}", ows(ws_num, set), colored)
        }
    };

    let mut text = String::new();
    for set in &seen_sets {
        let display_set = if set.is_empty() { "default" } else { set.as_str() };
        text.push_str(&format!("%{{A1:{}:}}{}:%{{A}}", ch_st(display_set), display_set));

        let mut st_locals: Vec<i64> = Vec::new();
        let mut st_globals: Vec<i64> = Vec::new();
        for i in 0..sets.len() {
            if &sets[i] == set {
                st_locals.push(locals[i]);
                st_globals.push(globals[i]);
            }
        }

        let local_count = st_locals.len();
        if local_count == 0 {
            text.push_str(" | ");
            continue;
        }
        let last_local = st_locals[local_count - 1];
        let mut prev_local = st_locals[0];
        let mut prev_long = st_globals[0];
        let mut seq_count: i64 = 1;

        for j in 0..local_count {
            let ws_num = st_locals[j];
            let ws_long = st_globals[j];

            if ws_num == current_local && set == &current_set {
                text.push_str(&format!(
                    "%{{u#a3be8c}}%{{+u}} %{{F{}}}{}%{{F-}} %{{-u}}",
                    opts.color_current_ws,
                    txt_item(&ws_num.to_string(), display_set, ws_num, ws_long)
                ));
            } else if set == &current_set || !opts.shorthand {
                text.push_str(&txt_item(
                    &format!(" {} ", ws_num),
                    display_set,
                    ws_num,
                    ws_long,
                ));
            } else if j == 0 {
                text.push_str(&txt_item(
                    &format!(" {}", ws_num),
                    display_set,
                    ws_num,
                    ws_long,
                ));
            } else if ws_num > prev_local + 1 {
                if seq_count > 2 {
                    text.push_str(&txt_item(
                        &format!("-{}", prev_local),
                        display_set,
                        prev_local,
                        prev_long,
                    ));
                } else if seq_count == 2 {
                    text.push_str(&txt_item(
                        &format!(", {}", prev_local),
                        display_set,
                        prev_local,
                        prev_long,
                    ));
                }
                text.push_str(&txt_item(
                    &format!(", {}", ws_num),
                    display_set,
                    ws_num,
                    ws_long,
                ));
                seq_count = 1;
            } else if ws_num == last_local {
                if seq_count > 1 {
                    text.push_str(&txt_item(
                        &format!("-{}", ws_num),
                        display_set,
                        ws_num,
                        ws_long,
                    ));
                } else {
                    text.push_str(&txt_item(
                        &format!(", {}", ws_num),
                        display_set,
                        ws_num,
                        ws_long,
                    ));
                }
            } else {
                seq_count += 1;
            }
            prev_local = ws_num;
            prev_long = ws_long;
        }
        text.push_str(" | ");
    }

    if let Some(stripped) = text.strip_suffix(" | ") {
        text = stripped.to_string();
    }
    Ok(text)
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
    if detect_wm() == "sway" {
        "swaymsg"
    } else {
        "i3-msg"
    }
}

// ── arg parsing helpers ───────────────────────────────────────────────────────

fn parse_set_context(args: &[String]) -> (SetContext, Vec<String>) {
    let mut ctx = SetContext::None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--group-active" => {
                ctx = SetContext::Active;
            }
            "--group-focused" => {
                ctx = SetContext::Focused;
            }
            "--group-name" => {
                i += 1;
                if i < args.len() {
                    ctx = SetContext::Named(args[i].clone());
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

    let mut conn =
        Connection::new().map_err(|e| format!("error: could not connect to i3/sway: {}", e))?;

    let result = match cmd {
        "switch-rewind" => cmd_switch_rewind(&mut conn),
        "list-groups" => {
            let monitor_only = has_flag(&rest, "--focused-monitor-only");
            cmd_list_groups(&mut conn, monitor_only)
        }
        "list-workspaces" => {
            let default_fields = "global_number,set,static_name,dynamic_name,local_number,global_name,monitor,focused,window_icons";
            let fields_str = get_flag_value(&rest, "--fields").unwrap_or(default_fields);
            let fields: Vec<&str> = fields_str.split(',').collect();
            let focused_only = has_flag(&rest, "--focused-only");
            let focused_monitor_only = has_flag(&rest, "--focused-monitor-only");
            let (set_ctx, _) = parse_set_context(&rest);
            cmd_list_workspaces(
                &mut conn,
                &fields,
                focused_only,
                focused_monitor_only,
                &set_ctx,
            )
        }
        "workspace-number" => {
            let (set_ctx, positional) = parse_set_context(&rest);
            let no_back_forth = has_flag(&rest, "--no-auto-back-and-forth");
            let number: i64 = positional
                .iter()
                .find(|a| !a.starts_with('-'))
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "error: workspace-number requires a number".to_string())?;
            cmd_workspace_number(&mut conn, number, &set_ctx, !no_back_forth)
        }
        "workspace-next" => cmd_workspace_relative(&mut conn, 1),
        "workspace-prev" => cmd_workspace_relative(&mut conn, -1),
        "next" | "workspace-next-global" => cmd_workspace_global_relative(&mut conn, 1),
        "prev" | "workspace-prev-global" => cmd_workspace_global_relative(&mut conn, -1),
        "workspace-new" => {
            let (set_ctx, _) = parse_set_context(&rest);
            cmd_workspace_new(&mut conn, &set_ctx)
        }
        "move-to-number" => {
            let (set_ctx, positional) = parse_set_context(&rest);
            let no_back_forth = has_flag(&rest, "--no-auto-back-and-forth");
            let number: i64 = positional
                .iter()
                .find(|a| !a.starts_with('-'))
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| "error: move-to-number requires a number".to_string())?;
            cmd_move_to_number(&mut conn, number, &set_ctx, no_back_forth)
        }
        "move-to-next" => cmd_move_relative(&mut conn, 1),
        "move-to-prev" => cmd_move_relative(&mut conn, -1),
        "move-to-new" => {
            let (set_ctx, _) = parse_set_context(&rest);
            cmd_move_to_new(&mut conn, &set_ctx)
        }
        "switch-active-group" => {
            let focused_monitor_only = has_flag(&rest, "--focused-monitor-only");
            let group = rest
                .iter()
                .find(|a| !a.starts_with('-'))
                .cloned()
                .unwrap_or_default();
            cmd_switch_active_group(&mut conn, &group, focused_monitor_only)
        }
        "rename-workspace" => {
            let name = get_flag_value(&rest, "--name").map(str::to_string);
            let number = get_flag_value(&rest, "--number").and_then(|s| s.parse::<i64>().ok());
            let set = get_flag_value(&rest, "--group").map(str::to_string);
            cmd_rename_workspace(&mut conn, name.as_deref(), number, set.as_deref())
        }
        "assign-workspace-to-group" => {
            let group = rest
                .iter()
                .find(|a| !a.starts_with('-'))
                .cloned()
                .unwrap_or_default();
            cmd_assign_workspace_to_group(&mut conn, &group)
        }
        "waybar" => cmd_waybar(&mut conn),
        "init-session" => cmd_init_session(&mut conn),
        "polybar" => {
            let opts = parse_polybar_opts(&rest);
            return cmd_polybar(&mut conn, &opts).map_err(|e| format!("error: {}", e));
        }
        "select-group" => {
            let mesg = get_flag_value(&rest, "-mesg").unwrap_or("").to_string();
            return cmd_select_set(&mut conn, &mesg).map(|s| s.unwrap_or_default());
        }
        "switch-group" => return cmd_switch_set(&mut conn),
        "assign" => return cmd_assign_menu(&mut conn),
        "assign-switch" => return cmd_assign_switch_menu(&mut conn),
        "focus" => return cmd_focus_menu(&mut conn),
        "move" => return cmd_move_menu(&mut conn),
        "rename" => return cmd_rename_menu(&mut conn),
        _ => return Err(format!("error: unknown command: {}", cmd)),
    };

    result.map_err(|e| format!("error: {}", e))
}

// ── doctor ───────────────────────────────────────────────────────────────────

const VALID_CMDS: &[&str] = &[
    "detect-wm", "wm-msg", "bar-updater", "doctor",
    "switch-rewind",
    "list-groups", "list-workspaces",
    "workspace-number", "workspace-next", "workspace-prev",
    "next", "prev", "workspace-next-global", "workspace-prev-global",
    "workspace-new",
    "move-to-number", "move-to-next", "move-to-prev", "move-to-new",
    "switch-active-group",
    "rename-workspace",
    "assign-workspace-to-group",
    "waybar", "init-session", "polybar",
    "select-group",
    "switch-group", "assign", "assign-switch", "focus", "move", "rename",
];

// (deprecated, replacement hint)
const RENAMED_CMDS: &[(&str, &str)] = &[
    ("list-sets",                      "list-groups"),
    ("list-workspace-groups",          "list-groups"),
    ("switch-active-set",              "switch-active-group"),
    ("assign-workspace-to-set",        "assign-workspace-to-group"),
    ("select-set",                     "select-group"),
    ("switch-set",                     "switch-group"),
    ("switch-to-last-workspace",       "switch-rewind"),
    ("workspace-number-focused-group", "workspace-number --group-focused <N>"),
    ("move-to-number-focused-group",   "move-to-number --group-focused <N>"),
];

const OLD_BINS: &[&str] = &[
    "i3-workspace-sets",
    "i3wsgroups",
    "i3-switch-active-workspace-set",
    "i3-assign-workspace-to-set",
    "i3-focus-on-workspace",
    "i3-move-to-workspace",
    "i3-rename-workspace",
    "i3-sets-waybar-module",
    "i3-sets-bar-module-updater",
    "i3-sets-polybar-module-updater",
];

struct Dr {
    pass: usize,
    warn: usize,
    fail: usize,
    color: bool,
}

impl Dr {
    fn new() -> Self {
        let color = env::var("NO_COLOR").is_err()
            && env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
        Self { pass: 0, warn: 0, fail: 0, color }
    }
    fn esc<'a>(&self, code: &'a str) -> &'a str {
        if self.color { code } else { "" }
    }
    fn ok(&mut self, msg: &str) {
        println!("  {}[OK]{}   {}", self.esc("\x1b[0;32m"), self.esc("\x1b[0m"), msg);
        self.pass += 1;
    }
    fn warn(&mut self, msg: &str) {
        println!("  {}[WARN]{} {}", self.esc("\x1b[0;33m"), self.esc("\x1b[0m"), msg);
        self.warn += 1;
    }
    fn fail(&mut self, msg: &str) {
        println!("  {}[FAIL]{} {}", self.esc("\x1b[0;31m"), self.esc("\x1b[0m"), msg);
        self.fail += 1;
    }
    fn info(&self, msg: &str) {
        println!("       {}", msg);
    }
    fn section(&self, title: &str) {
        let bar: String = "─".repeat(title.chars().count());
        println!("\n{}{}{}\n{}", self.esc("\x1b[1m"), title, self.esc("\x1b[0m"), bar);
    }
    fn summary(&self) {
        print!("\n{}Summary{}: ", self.esc("\x1b[1m"), self.esc("\x1b[0m"));
        if self.fail > 0 {
            print!("{}{} failed{}  ", self.esc("\x1b[0;31m"), self.fail, self.esc("\x1b[0m"));
        }
        if self.warn > 0 {
            print!("{}{} warnings{}  ", self.esc("\x1b[0;33m"), self.warn, self.esc("\x1b[0m"));
        }
        if self.pass > 0 {
            print!("{}{} passed{}", self.esc("\x1b[0;32m"), self.pass, self.esc("\x1b[0m"));
        }
        println!();
    }
    fn exit_code(&self) -> i32 {
        if self.fail > 0 { 2 } else if self.warn > 0 { 1 } else { 0 }
    }
}

fn dr_find_in_path(name: &str) -> Option<String> {
    let path_var = env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        if dir.is_empty() { continue; }
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn dr_find_config(override_path: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(p) = override_path {
        let path = Path::new(p);
        if path.exists() { return Some(path.to_path_buf()); }
        eprintln!("doctor: config not found: {}", p);
        return None;
    }
    let home = env::var("HOME").unwrap_or_default();
    let xdg = env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", home));

    let mut candidates: Vec<String> = Vec::new();
    if env::var("SWAYSOCK").is_ok() {
        candidates.push(format!("{}/sway/config", xdg));
    }
    if env::var("I3SOCK").is_ok() {
        candidates.push(format!("{}/i3/config", xdg));
    }
    candidates.push(format!("{}/sway/config", xdg));
    candidates.push(format!("{}/i3/config", xdg));
    if !home.is_empty() {
        candidates.push(format!("{}/.sway/config", home));
        candidates.push(format!("{}/.i3/config", home));
    }
    for c in candidates {
        let p = Path::new(&c);
        if p.exists() { return Some(p.to_path_buf()); }
    }
    None
}

fn dr_read_glob(pattern: &str, out: &mut Vec<String>) {
    if let Some(star) = pattern.find('*') {
        let prefix = &pattern[..star];
        let dir_part = Path::new(prefix).parent().unwrap_or(Path::new(".")).to_path_buf();
        let suffix = pattern[star + 1..].trim_start_matches('/');
        if let Ok(entries) = std::fs::read_dir(&dir_part) {
            let mut paths: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let n = e.file_name();
                    let s = n.to_string_lossy();
                    !s.starts_with('.') && (suffix.is_empty() || s.ends_with(suffix))
                })
                .map(|e| e.path())
                .collect();
            paths.sort();
            for p in paths {
                if let Ok(c) = std::fs::read_to_string(&p) {
                    out.extend(c.lines().map(String::from));
                }
            }
        }
    } else if let Ok(c) = std::fs::read_to_string(pattern) {
        out.extend(c.lines().map(String::from));
    }
}

fn dr_gather_lines(config: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let dir = config.parent().unwrap_or(Path::new(".")).to_path_buf();
    let home = env::var("HOME").unwrap_or_default();
    let content = match std::fs::read_to_string(config) {
        Ok(c) => c,
        Err(_) => return out,
    };
    for line in content.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("include ") {
            let pattern = rest.trim().trim_matches('"');
            let resolved = if pattern.starts_with("~/") {
                format!("{}{}", home, &pattern[1..])
            } else if pattern.starts_with('/') {
                pattern.to_string()
            } else {
                format!("{}/{}", dir.display(), pattern)
            };
            dr_read_glob(&resolved, &mut out);
        } else {
            out.push(line.to_string());
        }
    }
    out
}

fn dr_collect_aliases(lines: &[String]) -> Vec<String> {
    let mut aliases = Vec::new();
    for line in lines {
        let t = line.trim();
        if !t.starts_with("set ") { continue; }
        let tokens: Vec<&str> = t.split_whitespace().collect();
        if tokens.len() < 3 { continue; }
        let varname = tokens[1];
        if !varname.starts_with('$') { continue; }
        let var_pos = t.find(varname).unwrap_or(0);
        let after_var = t[var_pos + varname.len()..].trim();
        if after_var == "exec swi3-groups"
            || after_var == "exec --no-startup-id swi3-groups"
            || after_var.starts_with("exec swi3-groups ")
            || after_var.starts_with("exec --no-startup-id swi3-groups ")
        {
            aliases.push(varname.to_string());
        }
    }
    aliases
}

fn dr_extract_cmd(line: &str, aliases: &[String]) -> Option<String> {
    let t = line.trim();
    if t.starts_with('#') || t.is_empty() || t.starts_with("set ") {
        return None;
    }
    for sep in ["exec --no-startup-id swi3-groups ", "exec swi3-groups "] {
        if let Some(pos) = t.find(sep) {
            let before = &t[..pos];
            if !before.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                let after = t[pos + sep.len()..].trim_start();
                if let Some(cmd) = after.split_whitespace().next() {
                    if !cmd.starts_with('-') {
                        return Some(cmd.to_string());
                    }
                }
            }
        }
    }
    for alias in aliases {
        if let Some(pos) = t.find(alias.as_str()) {
            let before = &t[..pos];
            if before.is_empty() || before.ends_with(|c: char| c.is_whitespace()) {
                let after = t[pos + alias.len()..].trim_start();
                if let Some(cmd) = after.split_whitespace().next() {
                    if !cmd.starts_with('-') && !cmd.starts_with('$') {
                        return Some(cmd.to_string());
                    }
                }
            }
        }
    }
    None
}

fn cmd_doctor(config_path: Option<&str>) -> i32 {
    let mut d = Dr::new();
    println!("{}swi3-groups doctor{}", d.esc("\x1b[1m"), d.esc("\x1b[0m"));

    // ── 1. Binaries ───────────────────────────────────────────────────────────
    d.section("1. Binaries");

    match dr_find_in_path("swi3-groups") {
        Some(p) => d.ok(&format!("swi3-groups found: {}", p)),
        None => {
            d.fail("swi3-groups not found in PATH");
            d.info("Run 'make install' to install it.");
        }
    }

    // ── 2. Config file ────────────────────────────────────────────────────────
    d.section("2. Config file");

    let cfg_path = match dr_find_config(config_path) {
        Some(p) => {
            d.ok(&format!("Config: {}", p.display()));
            p
        }
        None => {
            d.fail("No sway/i3 config file found");
            d.info("Searched $XDG_CONFIG_HOME/sway|i3/config and ~/.sway|i3/config");
            d.summary();
            return d.exit_code();
        }
    };

    let lines = dr_gather_lines(&cfg_path);
    let aliases = dr_collect_aliases(&lines);

    // ── 3. Command usage ──────────────────────────────────────────────────────
    d.section("3. Command usage");

    let mut seen_bad: HashSet<String> = HashSet::new();
    let mut any_bad = false;

    for line in &lines {
        let Some(cmd) = dr_extract_cmd(line, &aliases) else { continue };

        if let Some(&(_, suggestion)) =
            RENAMED_CMDS.iter().find(|(old, _)| *old == cmd.as_str())
        {
            if seen_bad.insert(cmd.clone()) {
                d.fail(&format!("Invalid command '{}'", cmd));
                d.info(&format!("→ Replace with: {}", suggestion));
                d.info(&format!("  Context: {}", line.trim()));
                any_bad = true;
            }
            continue;
        }

        if !VALID_CMDS.contains(&cmd.as_str()) && seen_bad.insert(cmd.clone()) {
            d.fail(&format!("Unknown command '{}'", cmd));
            d.info(&format!("  Context: {}", line.trim()));
            any_bad = true;
        }
    }
    if !any_bad {
        d.ok("All swi3-groups commands are valid");
    }

    // ── 4. Deprecated binary names ────────────────────────────────────────────
    d.section("4. Deprecated binary names");

    let mut old_count = 0;
    for old in OLD_BINS {
        let found = lines
            .iter()
            .any(|l| !l.trim().starts_with('#') && l.contains(old));
        if found {
            d.fail(&format!("Old binary '{}' still referenced in config", old));
            d.info("→ Replace with 'swi3-groups'");
            old_count += 1;
        }
    }
    if old_count == 0 {
        d.ok("No deprecated binary names found");
    }

    // ── 5. init-session ───────────────────────────────────────────────────────
    d.section("5. Session init");

    let has_init = lines
        .iter()
        .any(|l| dr_extract_cmd(l, &aliases).as_deref() == Some("init-session"));
    if has_init {
        d.ok("init-session is exec'd on startup");
    } else {
        d.warn("init-session not found in config");
        d.info("Add to your config for proper workspace numbering on first login:");
        d.info("  exec swi3-groups init-session");
    }

    // ── 6. Raw workspace navigation ───────────────────────────────────────────
    d.section("6. Workspace navigation");

    let mut raw_count = 0;
    for line in &lines {
        let t = line.trim();
        if t.starts_with('#') { continue; }
        if t.contains("bindsym")
            && !t.contains("swi3-groups")
            && (t.contains("workspace next") || t.contains("workspace prev"))
            && !t.contains("next_on_output")
            && !t.contains("prev_on_output")
        {
            d.warn("Raw 'workspace next/prev' binding bypasses group navigation");
            d.info("  Consider: swi3-groups next / swi3-groups prev");
            d.info(&format!("  Context: {}", t));
            raw_count += 1;
        }
    }
    if raw_count == 0 {
        d.ok("No raw workspace next/prev bindings found");
    }

    // ── 7. Bar integration ────────────────────────────────────────────────────
    d.section("7. Bar integration");

    let uses_swaybar = lines
        .iter()
        .any(|l| matches!(l.trim(), "bar {" | "bar{"));
    let uses_waybar = lines.iter().any(|l| {
        let t = l.trim();
        !t.starts_with('#') && t.contains("exec") && t.contains("waybar")
    });

    let mut bar_checked = false;

    if uses_swaybar {
        bar_checked = true;
        let has_strip = lines.iter().any(|l| {
            let t = l.trim();
            !t.starts_with('#') && t.contains("strip_workspace_numbers") && t.contains("yes")
        });
        if has_strip {
            d.ok("swaybar: strip_workspace_numbers yes");
        } else {
            d.warn("swaybar: 'strip_workspace_numbers yes' not set in bar { } block");
            d.info("Add it to avoid showing raw encoded workspace names.");
        }
    }

    if uses_waybar {
        bar_checked = true;
        let has_updater = lines.iter().any(|l| {
            let t = l.trim();
            !t.starts_with('#')
                && t.contains("exec")
                && t.contains("bar-updater")
                && (t.contains("swi3-groups") || t.contains("i3-sets-bar-module-updater"))
        });
        if has_updater {
            d.ok("waybar: bar-updater exec'd");
        } else {
            d.warn("waybar detected but 'swi3-groups bar-updater' not found in config");
            d.info("Add: exec swi3-groups bar-updater");
        }
    }

    if !bar_checked {
        d.info("No swaybar or waybar config detected");
    }

    // ── 8. Menu backend ───────────────────────────────────────────────────────
    d.section("8. Menu backend");

    if let Ok(custom) = env::var("SWI3SETS_MENU") {
        if !custom.trim().is_empty() {
            d.ok(&format!("Menu overridden via $SWI3SETS_MENU: {}", custom.trim()));
        }
    } else {
        let found = ["rofi", "wofi", "fuzzel", "dmenu"]
            .iter()
            .find(|&&m| dr_find_in_path(m).is_some())
            .copied();
        match found {
            Some(m) => d.ok(&format!("Menu backend available: {}", m)),
            None => {
                d.warn("No menu backend found (rofi, wofi, fuzzel, dmenu)");
                d.info(
                    "Interactive commands (switch-group, assign, focus, move, rename) will not work.",
                );
                d.info("Install one, or set $SWI3SETS_MENU to a custom launcher.");
            }
        }
    }

    d.summary();
    d.exit_code()
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if let Some(first) = args.first() {
        match first.as_str() {
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
            "doctor" => {
                let config = args
                    .iter()
                    .position(|a| a == "--config" || a == "-c")
                    .and_then(|i| args.get(i + 1))
                    .map(|s| s.as_str());
                process::exit(cmd_doctor(config));
            }
            _ => {}
        }
    }

    match dispatch(&args) {
        Ok(response) => {
            if !response.is_empty() {
                print!("{}", response);
                if !response.ends_with('\n') {
                    println!();
                }
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}
