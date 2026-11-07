//! Read-aloud with the system's own voice. The script read is the outcome's
//! `spoken` text, written for listening, never the page.

use std::sync::{atomic::{AtomicU64, Ordering}, Mutex};
use tokio::{process::{Child, Command}, sync::oneshot};

#[derive(Default)]
pub struct Speech {
    stop: Mutex<Option<oneshot::Sender<()>>>,
    generation: AtomicU64,
}

impl Speech {
    /// Resolve at the real end, so the page's Stop reading state matches audio.
    pub async fn speak(&self, text: &str) -> Result<(), String> {
        self.stop().await;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let child = start(text).await?;
        let (send, cancel) = oneshot::channel();
        {
            let mut stop = self.stop.lock().unwrap();
            if self.generation.load(Ordering::SeqCst) == generation { *stop = Some(send); }
            // A stop during startup drops send; cancel immediately resolves.
        }
        finish(child, cancel).await?;
        { let mut stop = self.stop.lock().unwrap(); if self.generation.load(Ordering::SeqCst) == generation { stop.take(); } }
        Ok(())
    }

    pub async fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let cancel = self.stop.lock().unwrap().take();
        if let Some(cancel) = cancel { let _ = cancel.send(()); }
        #[cfg(target_os = "linux")]
        { let _ = Command::new("spd-say").arg("-C").status().await; }
    }
}

async fn finish(mut child: Child, cancel: oneshot::Receiver<()>) -> Result<(), String> {
    tokio::select! {
        result = child.wait() => { result.map_err(|e| e.to_string())?; },
        _ = cancel => { let _ = child.kill().await; },
    }
    Ok(())
}

/// Report only an installed executable, without speaking as a probe.
pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    let names = ["spd-say", "espeak-ng", "espeak"];
    #[cfg(target_os = "windows")]
    let names = ["powershell.exe"];
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let names: [&str; 0] = [];
    std::env::var_os("PATH").map(|path| std::env::split_paths(&path).any(|dir| names.iter().any(|name| executable(&dir.join(name))))).unwrap_or(false)
}

fn executable(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else { return false };
    if !metadata.is_file() { return false; }
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; metadata.permissions().mode() & 0o111 != 0 }
    #[cfg(not(unix))]
    { true }
}

#[cfg(target_os = "linux")]
async fn start(text: &str) -> Result<Child, String> {
    // The text is an argument, never a shell string.
    for (prog, args) in [("spd-say", vec!["-w", "-e"]), ("espeak-ng", vec!["--stdin"]), ("espeak", vec!["--stdin"])] {
        let spawned = Command::new(prog).args(&args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn();
        if let Ok(mut c) = spawned {
            use tokio::io::AsyncWriteExt;
            if let Some(mut i) = c.stdin.take() {
                let _ = i.write_all(text.as_bytes()).await;
            }
            return Ok(c);
        }
    }
    Err("No speech program is installed. Install speech-dispatcher or espeak-ng.".into())
}

#[cfg(target_os = "windows")]
async fn start(text: &str) -> Result<Child, String> {
    use tokio::io::AsyncWriteExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak([Console]::In.ReadToEnd())"])
        .stdin(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("start speech: {e}"))?;
    if let Some(mut i) = c.stdin.take() {
        let _ = i.write_all(text.as_bytes()).await;
    }
    Ok(c)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
async fn start(_: &str) -> Result<Child, String> {
    Err("Read-aloud is not available on this system.".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn playback_completion_waits_for_the_child_and_stop_reaps_it() {
        let child = Command::new("sleep").arg("0.05").spawn().unwrap();
        let pid = child.id().unwrap();
        let (_sender, receiver) = oneshot::channel();
        finish(child, receiver).await.unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id().unwrap();
        let (sender, receiver) = oneshot::channel();
        sender.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), finish(child, receiver)).await.unwrap().unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
    #[test]
    fn speech_capability_requires_an_executable_file() {
        assert!(executable(std::path::Path::new("/bin/sh")));
        assert!(!executable(std::path::Path::new("/etc/passwd")));
        assert!(!executable(std::path::Path::new("/bin")));
    }
}
