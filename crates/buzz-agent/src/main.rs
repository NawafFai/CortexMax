fn main() {
    if std::env::args().nth(1).as_deref() == Some("--codex-project-bridge-capabilities") {
        println!("buzz-codex-reasoning-broker-v1");
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--native-project-runtime-capabilities") {
        println!("buzz-native-project-text-v1");
        return;
    }
    if let Some(result) = buzz_agent::project_workspace::run_file_helper_if_requested() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(result) = buzz_agent::project_workspace::task_mcp::serve_if_requested() {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = buzz_agent::run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
