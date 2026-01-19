use std::process::{Command, Child, Stdio};
use std::time::{Duration, Instant};
use std::net::TcpStream;
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

    pub fn spawn_server(&mut self, shards: usize, capacity: usize, port: u16) -> Result<()> {
        let mut child = Command::new("./target/release/vortex-server")
            .arg("--shards")
            .arg(shards.to_string())
            .arg("--capacity")
            .arg(capacity.to_string())
            .arg("--port")
            .arg(port.to_string())
            .env("RUST_LOG", "vortex_core=info,vortex_server=info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn vortex-server")?;
        
        self.server_stderr = child.stderr.take();
        self.server_process = Some(child);
        Ok(())
    }

    pub fn wait_for_readiness(&self, port: u16, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let addr = format!("127.0.0.1:{}", port);
        
        while start.elapsed() < timeout {
            if TcpStream::connect(&addr).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        
        Err(anyhow::anyhow!("Readiness timeout: VORTEX failed to open port {} after {:?}", port, timeout))
    }


    pub fn kill_all(&mut self) {
        if let Some(mut child) = self.server_process.take() {
            let _ = child.kill();
        }
    }
}
