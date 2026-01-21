use std::process::{Command, Child, Stdio};
// use std::time::{Duration, Instant};
// use std::net::TcpStream;
use sysinfo::System;
use anyhow::{Result, Context};
use std::fs;

pub struct LifecycleManager {
    pub server_process: Option<Child>,
    pub server_stderr: Option<std::process::ChildStderr>,
}

impl LifecycleManager {
    pub fn new() -> Self {
        Self {
            server_process: None,
            server_stderr: None,
        }
    }

    pub fn cleanup_zombies(&self) {
        let mut sys = System::new_all();
        sys.refresh_all();
        
        for (_pid, process) in sys.processes() {
            if process.name().contains("vortex-server") {
                process.kill();
            }
        }
    }

    pub fn clean_data_dir(&self, path: &str) -> Result<()> {
        if fs::metadata(path).is_ok() {
            fs::remove_dir_all(path).context("Failed to clean data directory")?;
        }
        fs::create_dir_all(path).context("Failed to recreate data directory")?;
        Ok(())
    }

    pub fn spawn_server(&mut self, args: &crate::Args) -> Result<()> {
        let mut child = Command::new("./target/release/vortex-server")
            .arg("--shards")
            .arg(args.shards.to_string())
            .arg("--capacity")
            .arg(args.capacity.to_string())
            .arg("--port")
            .arg(args.port.to_string())
            .arg("--dir")
            .arg(&args.dir)
            .env("RUST_LOG", "info")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn vortex-server")?;
        
        self.server_stderr = child.stderr.take();
        self.server_process = Some(child);
        Ok(())
    }

}
