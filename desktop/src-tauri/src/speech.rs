//! Read-aloud with the system's own voice. The script read is the outcome's
//! `spoken` text, written for listening, never the page.

use std::sync::Mutex;
use tokio::process::{Child, Command};

#[derive(Default)]
pub struct Speech(Mutex<Option<Child>>);

impl Speech {
    pub async fn speak(&self, text: &str) -> Result<(), String> {
        self.stop().await;
        let child = start(text).await?;
        *self.0.lock().unwrap() = Some(child);
        Ok(())
    }

    pub async fn stop(&self) {
        let child = self.0.lock().unwrap().take();
        if let Some(mut c) = child {
            let _ = c.start_kill();
            #[cfg(target_os = "linux")]
            {
                // speech-dispatcher keeps speaking after its client exits.
                let _ = Command::new("spd-say").arg("-C").status().await;
            }
        }
    }
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
