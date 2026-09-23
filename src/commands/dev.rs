use colored::*;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use notify::{event::ModifyKind, Event, RecursiveMode, Watcher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use std::env;

use super::output;

/// Find the project root by searching for `toxi.toml` or `Cargo.toml`
/// starting from the current directory and walking up the tree.
fn find_project_root() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        if current.join("toxi.toml").exists() || current.join("Cargo.toml").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Load `.env` from the project root, if present.
fn load_dotenv() {
    if env::var("TOXI_SKIP_DOTENV").is_err() {
        if let Some(root) = find_project_root() {
            let env_path = root.join(".env");
            if env_path.exists() {
                let _ = dotenv::from_path(&env_path);
                return;
            }
        }
        let _ = dotenv::dotenv();
    }
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub env: Option<String>,
    pub bin: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DevOptions {
    pub run: RunOptions,
    pub watch: Vec<PathBuf>,
    pub ignore: Vec<String>,
    pub hot_reload: bool,
}

pub fn run_project_once(
    release: bool,
    options: &RunOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    load_dotenv();
    output::debug("Loading environment variables from .env");

    if release {
        output::info("Starting Toxi server in release mode");
    } else {
        output::info("Starting Toxi project in debug mode");
    }
    output::debug(&format!("Server options: {:?}", options));

    let mut child = spawn_cargo_run(release, options)?;
    let status = child.wait()?;
    if status.success() {
        output::success("Process completed successfully");
        Ok(())
    } else {
        Err(format!("process exited with status {status}").into())
    }
}

pub fn start_dev_server(options: DevOptions) -> Result<(), Box<dyn std::error::Error>> {
    load_dotenv();

    if !options.hot_reload {
        output::warning("Hot reload disabled; running project once");
        return run_project_once(false, &options.run);
    }

    let project_root = find_project_root()
        .ok_or("No Cargo.toml found in current or parent directories")?;

    // Determine binary name and path
    let (bin_name, binary_path) = resolve_binary_path(&project_root, &options.run);
    output::debug(&format!("Binary target: {}", bin_name));
    output::debug(&format!("Binary path: {}", binary_path.display()));

    output::success("Starting Toxi development server");
    output::info("Watching for file changes");

    // Build once, then start the server when the build succeeds. A failed
    // initial build must not terminate the watcher, since the user corrects
    // compilation errors while the watcher remains active and the server
    // starts upon the first successful rebuild.
    let child_process: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    if build_project(&project_root, &bin_name)? {
        start_binary(&child_process, &binary_path, &options.run)?;
    } else {
        output::error("Initial build failed; watching for fixes. The server will start once the build succeeds.");
    }

    let watch_paths = if options.watch.is_empty() {
        default_watch_paths(&project_root)
    } else {
        options.watch.clone()
    };
    let ignore_patterns = default_ignore_patterns(&options.ignore);

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;

    for path in &watch_paths {
        if path.exists() {
            watcher.watch(path, RecursiveMode::Recursive)?;
        } else {
            println!("⚠️  Watch path not found: {}", path.display());
        }
    }

    // State for the compile-ahead loop
    let should_rebuild: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let build_in_progress: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

    let mut last_restart = Instant::now();
    let debounce_duration = Duration::from_millis(200);

    loop {
        // Poll with a timeout. A change that lands mid-build must rebuild
        // once the build finishes. Blocking recv would wait for another file
        // change that may never come, and that rebuild is lost.
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(event)) => {
                if should_reload(&event, &ignore_patterns) {
                    let now = Instant::now();
                    if now.duration_since(last_restart) > debounce_duration {
                        *should_rebuild.lock().unwrap() = true;
                        last_restart = now;
                    }
                }
            }
            Ok(Err(err)) => println!("Watch error: {err:?}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Report a server that exited on its own without terminating the
        // watcher, since the user corrects runtime failures in the same
        // manner as compilation failures while the watcher remains active.
        // The slot is cleared so the next successful build starts a fresh
        // process rather than attempting a graceful swap with a dead child.
        {
            let mut guard = child_process.lock().unwrap();
            if let Some(ref mut child) = *guard {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        if status.success() {
                            output::info("Server exited; waiting for changes.");
                        } else {
                            output::error(&format!(
                                "Server exited with status {status}; waiting for changes."
                            ));
                        }
                        *guard = None;
                    }
                    Ok(None) => {}
                    Err(err) => {
                        output::error(&format!(
                            "Failed to poll server status ({err}); waiting for changes."
                        ));
                        *guard = None;
                    }
                }
            }
        }

        // Try to rebuild if requested and not already building
        let rebuild_requested = *should_rebuild.lock().unwrap();
        let already_building = *build_in_progress.lock().unwrap();

        if rebuild_requested && !already_building {
            *should_rebuild.lock().unwrap() = false;
            *build_in_progress.lock().unwrap() = true;

            println!("\n{}", "Changes detected, rebuilding...".yellow());

            // Spawn a build thread so the old server keeps running
            let root = project_root.clone();
            let bname = bin_name.clone();
            let child_lock = child_process.clone();
            let bpath = binary_path.clone();
            let run_opts = options.run.clone();
            let build_flag = build_in_progress.clone();

            thread::spawn(move || {
                let success = build_project(&root, &bname).unwrap_or(false);
                if success {
                    // When no server is running, which follows an initial
                    // build failure or a server crash, the fresh binary is
                    // started directly. Otherwise a graceful swap is
                    // performed through SIGTERM with a wait before restart.
                    let had_server = child_lock.lock().unwrap().is_some();
                    graceful_stop(&child_lock);
                    match start_binary(&child_lock, &bpath, &run_opts) {
                        Ok(()) => {
                            if had_server {
                                println!("{}", "Server restarted with new code.".green());
                            } else {
                                println!("{}", "Build succeeded; server started.".green());
                            }
                        }
                        Err(err) => {
                            println!(
                                "{}",
                                format!("Build succeeded but the server failed to start ({err}); waiting for changes.").yellow()
                            );
                        }
                    }
                } else if child_lock.lock().unwrap().is_some() {
                    println!(
                        "{}",
                        "Build failed, old server is still running.".yellow()
                    );
                } else {
                    println!(
                        "{}",
                        "Build failed; no server is running. Fix the errors and save to retry.".yellow()
                    );
                }
                *build_flag.lock().unwrap() = false;
            });
        }
    }

    Ok(())
}

/// Build the project binary. Returns true on success.
///
/// Standard output is inherited rather than piped, since a piped stream
/// that is never drained risks deadlock when the pipe buffer fills, with
/// the consequence that the build would stall while the watcher waits.
/// Standard error is piped and streamed line by line so that compilation
/// progress remains visible in realtime while the watcher remains active.
fn build_project(project_root: &Path, bin_name: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("--bin")
        .arg(bin_name)
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .current_dir(project_root);

    let mut child = cmd.spawn()?;

    // Stream stderr in real-time so the user sees compilation progress
    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        if let Ok(line) = line {
            eprintln!("{}", line);
        }
    }

    let status = child.wait()?;
    Ok(status.success())
}

/// Start the compiled binary in the background. Returns immediately.
fn start_binary(
    child_lock: &Arc<Mutex<Option<Child>>>,
    binary_path: &Path,
    options: &RunOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cmd = Command::new(binary_path);
    apply_run_env(&mut cmd, options);
    let child = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit())
        .spawn()?;
    *child_lock.lock().unwrap() = Some(child);
    Ok(())
}

/// Resolve the binary path from project metadata.
fn resolve_binary_path(project_root: &Path, options: &RunOptions) -> (String, PathBuf) {
    let bin_name = options
        .bin
        .clone()
        .unwrap_or_else(|| {
            // Read package name from Cargo.toml
            let cargo_toml = project_root.join("Cargo.toml");
            std::fs::read_to_string(&cargo_toml)
                .ok()
                .and_then(|content| package_name_from_manifest(&content))
                .unwrap_or_else(|| "app".to_string())
        });
    let binary_path = project_root.join("target").join("debug").join(&bin_name);
    (bin_name, binary_path)
}

/// Read [package] name from Cargo.toml text.
///
/// Only the [package] section counts. Workspace manifests have other name
/// keys that would resolve to the wrong binary.
fn package_name_from_manifest(content: &str) -> Option<String> {
    let mut in_package = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("name") {
            if let Some(value) = rest.trim_start().strip_prefix('=') {
                let value = value
                    .split('#')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Watch source and config paths only.
///
/// Watching the whole project root also watches target/ and .git/, so every
/// build fires file events and triggers another rebuild.
fn default_watch_paths(project_root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in ["src", "migrations", "seeds", "templates", "tests"] {
        let p = project_root.join(dir);
        if p.exists() {
            paths.push(p);
        }
    }
    for file in ["Cargo.toml", "Cargo.lock", "toxi.toml", ".env"] {
        let p = project_root.join(file);
        if p.exists() {
            paths.push(p);
        }
    }
    if paths.is_empty() {
        paths.push(project_root.to_path_buf());
    }
    paths
}

/// Gracefully stop a running process with SIGTERM, then SIGKILL if needed.
fn graceful_stop(child_lock: &Arc<Mutex<Option<Child>>>) {
    let mut lock = child_lock.lock().unwrap();
    if let Some(ref mut child) = *lock {
        let pid = child.id();

        // SIGTERM first so in-flight requests finish.
        // Toxi servers take up to 3s to drain, so wait 5s before killing.
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);

        // Wait up to 5s for graceful shutdown
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => {
                    *lock = None;
                    return;
                }
                Ok(None) => thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }

        // Force kill if still running
        let _ = child.kill();
        let _ = child.wait();
    }
    *lock = None;
}

/// Spawn `cargo run` (used by `serve` / `run_project_once`).
fn spawn_cargo_run(release: bool, options: &RunOptions) -> std::io::Result<Child> {
    let mut command = Command::new("cargo");
    command.arg("run");
    if release {
        command.arg("--release");
    }
    if let Some(bin) = &options.bin {
        command.arg("--bin").arg(bin);
    }
    apply_run_env(&mut command, options);
    command
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .stdin(Stdio::inherit())
        .spawn()
}

pub fn apply_run_env(command: &mut Command, options: &RunOptions) {
    if let Some(host) = &options.host {
        command.env("SERVER_HOST", host);
    }
    if let Some(port) = options.port {
        command.env("SERVER_PORT", port.to_string());
    }
    if let Some(environment) = &options.env {
        command.env("TOXI_ENV", environment);
    }
}

fn should_reload(event: &Event, ignore_patterns: &[String]) -> bool {
    let is_relevant_event = matches!(
        event.kind,
        notify::EventKind::Modify(ModifyKind::Data(_))
            | notify::EventKind::Create(_)
            | notify::EventKind::Remove(_)
    );
    if !is_relevant_event {
        return false;
    }

    for path in &event.paths {
        let path_str = path.to_string_lossy();
        if should_ignore_path(&path_str, ignore_patterns) {
            continue;
        }

        if is_reloadable_path(path) {
            return true;
        }
    }

    false
}

fn should_ignore_path(path: &str, ignore_patterns: &[String]) -> bool {
    if path.contains("/target/")
        || path.contains("\\target\\")
        || path.contains("/node_modules/")
        || path.contains("\\node_modules\\")
        || path.contains("/.git/")
        || path.contains("\\.git\\")
    {
        return true;
    }

    ignore_patterns
        .iter()
        .filter(|pattern| !pattern.is_empty())
        .any(|pattern| path.contains(pattern))
}

fn is_reloadable_path(path: &Path) -> bool {
    if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
        if matches!(
            file_name,
            "Cargo.toml" | "Cargo.lock" | "toxi.toml" | ".env"
        ) {
            return true;
        }
    }

    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "rs" | "toml" | "html" | "css" | "js" | "sql" | "md" | "yaml" | "yml" | "json" | "env"
        )
    )
}

fn default_ignore_patterns(extra: &[String]) -> Vec<String> {
    let mut patterns = vec![
        "/target/".to_string(),
        "\\target\\".to_string(),
        "/node_modules/".to_string(),
        "\\node_modules\\".to_string(),
        "/.git/".to_string(),
        "\\.git\\".to_string(),
    ];

    for pattern in extra {
        if !pattern.is_empty() && !patterns.contains(pattern) {
            patterns.push(pattern.clone());
        }
    }

    patterns
}

#[cfg(test)]
mod tests {
    use super::{default_ignore_patterns, default_watch_paths, is_reloadable_path, package_name_from_manifest, should_ignore_path};
    use std::path::Path;

    #[test]
    fn extends_ignore_patterns() {
        let patterns = default_ignore_patterns(&["dist".to_string()]);
        assert!(patterns.iter().any(|pattern| pattern == "dist"));
        assert!(patterns.iter().any(|pattern| pattern == "/target/"));
    }

    #[test]
    fn ignores_expected_paths() {
        let patterns = default_ignore_patterns(&[]);
        assert!(should_ignore_path("./target/debug/app", &patterns));
        assert!(should_ignore_path(
            "./dist/bundle.js",
            &["dist".to_string()]
        ));
        assert!(!should_ignore_path("./src/main.rs", &patterns));
    }

    #[test]
    fn marks_reloadable_files() {
        assert!(is_reloadable_path(Path::new("src/main.rs")));
        assert!(is_reloadable_path(Path::new("toxi.toml")));
        assert!(!is_reloadable_path(Path::new("README.txt")));
    }

    #[test]
    fn reads_package_name_only_from_package_section() {
        let manifest = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nname = \"workspace-name\"\n\n[package]\nname = \"real-bin\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            package_name_from_manifest(manifest).as_deref(),
            Some("real-bin")
        );
    }

    #[test]
    fn reads_package_name_without_spaces_and_comment() {
        let manifest = "[package]\nname=\"tight\" # trailing comment\n";
        assert_eq!(
            package_name_from_manifest(manifest).as_deref(),
            Some("tight")
        );
    }

    #[test]
    fn returns_none_without_package_section() {
        let manifest = "[workspace]\nmembers = []\n";
        assert_eq!(package_name_from_manifest(manifest), None);
    }

    #[test]
    fn default_watch_paths_prefers_src_over_root() {
        let root = std::env::temp_dir().join(format!(
            "toxi-dev-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n")
            .expect("write manifest");

        let paths = default_watch_paths(&root);
        assert!(paths.iter().any(|p| p.ends_with("src")));
        assert!(paths.iter().any(|p| p.ends_with("Cargo.toml")));
        assert!(!paths.iter().any(|p| p == &root));

        std::fs::remove_dir_all(&root).ok();
    }
}
