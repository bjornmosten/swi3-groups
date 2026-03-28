use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process;

fn socket_path() -> String {
    if let Ok(p) = env::var("I3_WORKSPACE_GROUPS_SOCKET") {
        return p;
    }
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    let display = env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
    let display_clean = display.replace(':', "");
    format!("{}/i3-workspace-groups-{}", runtime_dir, display_clean)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
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
