use std::{fs::OpenOptions, io::Write, path::PathBuf, process::Stdio, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

fn run_log_path() -> Option<Arc<PathBuf>> {
    let path = std::env::var_os("KAWARI_RUN_LOG_FILE").map(PathBuf::from)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("Failed to create log directory");
    }

    Some(Arc::new(path))
}

async fn copy_to_run_log<R>(mut reader: R, path: Arc<PathBuf>)
where
    R: AsyncRead + Unpin,
{
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&*path)
        .expect("Failed to open run log file");
    let mut buffer = [0u8; 8192];

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .expect("Failed to read server output");
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .expect("Failed to write server output to run log");
        file.flush()
            .expect("Failed to flush server output to run log");
    }
}

async fn start_server(name: &str, run_log_path: Option<Arc<PathBuf>>) {
    let mut dir = std::env::current_exe().expect("Couldn't get current executable path");
    dir.pop();

    let mut extension = std::env::consts::EXE_EXTENSION.to_string();
    if !extension.is_empty() {
        extension = format!(".{extension}");
    }

    dir.push(format!("{name}{}", extension));

    let library_path = if std::env::var("CARGO").is_ok() {
        "./oodle"
    } else {
        "."
    };

    let mut command = Command::new(dir);
    command
        .env("LD_LIBRARY_PATH", library_path) // ensure we find the oodle .so at the right location
        .env("RUST_BACKTRACE", "1"); // Print backtraces on asserts

    if run_log_path.is_some() {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    }

    let mut child = command.spawn().expect("Failed to run server");

    if let Some(path) = run_log_path {
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(copy_to_run_log(stdout, path.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(copy_to_run_log(stderr, path));
        }
    }

    child.wait().await.expect("Failed to run server");
}

#[tokio::main]
async fn main() {
    // Enables ANSI code support on Windows. See https://github.com/tokio-rs/tracing/issues/3068
    #[cfg(windows)]
    nu_ansi_term::enable_ansi_support().ok();

    // If being invoked by Cargo, build the workspace first.
    if let Ok(cargo) = std::env::var("CARGO") {
        let build_exit_status = Command::new(cargo)
            .args(if cfg!(debug_assertions) {
                vec!["build", "--features", "oodle"]
            } else {
                vec!["build", "--release", "--features", "oodle"]
            })
            .stdout(Stdio::inherit())
            .spawn()
            .expect("Failed to run Cargo build")
            .wait()
            .await
            .expect("Failed to run Cargo build");

        // Silently exit if build failed
        if !build_exit_status.success() {
            return;
        }
    }

    let run_log_path = run_log_path();

    tokio::join!(
        start_server("kawari-admin", run_log_path.clone()),
        start_server("kawari-frontier", run_log_path.clone()),
        start_server("kawari-launcher", run_log_path.clone()),
        start_server("kawari-lobby", run_log_path.clone()),
        start_server("kawari-login", run_log_path.clone()),
        start_server("kawari-patch", run_log_path.clone()),
        start_server("kawari-web", run_log_path.clone()),
        start_server("kawari-world", run_log_path.clone()),
    );
}
