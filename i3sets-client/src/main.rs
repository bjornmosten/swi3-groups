use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{self, Command};

fn detect_wm() -> &'static str {
    if env::var("SWAYSOCK").is_ok() {
        return "sway";
    }
    if env::var("I3SOCK").is_ok() {
        return "i3";
    }
    // Fallback: check if swaymsg is on PATH
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

fn display_id() -> String {
    let wm = detect_wm();
    if wm == "sway" {
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
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    format!("{}/i3-workspace-groups-{}", runtime_dir, display_id())
}

fn wm_msg_cmd() -> &'static str {
    if detect_wm() == "sway" {
        "swaymsg"
    } else {
        "i3-msg"
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    // Handle client-side subcommands (no server round-trip)
    if let Some(first) = args.first() {
        match first.as_str() {
            "detect-wm" => {
                println!("{}", detect_wm());
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

    let payload = args.join("\n");
    let path = socket_path();

    let mut stream = UnixStream::connect(&path).unwrap_or_else(|e| {
        eprintln!("error: could not connect to server at {}: {}", path, e);
        eprintln!("Start the server with: i3-workspace-groups server");
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
