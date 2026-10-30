//! Local speech to text with whisper.cpp, so voice never leaves the appliance.
//! The `[speech]` config names `whisper-cli` and a model; audio is converted
//! with ffmpeg. Read-aloud is the other way, with Kokoro (`[read_aloud]`).

use crate::config::{ReadAloud, Speech};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use std::path::Path;

pub async fn transcribe(speech: Option<&Speech>, state_dir: &Path, data_b64: &str) -> Result<String> {
    let cfg = speech.ok_or_else(|| anyhow!("voice input is not installed on this appliance"))?;
    let (bin, model) = (&cfg.whisper, &cfg.model);
    let audio = base64::engine::general_purpose::STANDARD.decode(data_b64).context("audio is not base64")?;
    let dir = state_dir.join("speech").join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&dir)?;
    let input = dir.join("in.audio");
    let wav = dir.join("in.wav");
    std::fs::write(&input, audio)?;
    let ff = tokio::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-i"])
        .arg(&input)
        .args(["-ar", "16000", "-ac", "1"])
        .arg(&wav)
        .output()
        .await
        .context("run ffmpeg")?;
    anyhow::ensure!(ff.status.success(), "could not read the audio: {}", String::from_utf8_lossy(&ff.stderr));
    let out = tokio::process::Command::new(&bin)
        .arg("-m")
        .arg(model)
        .args(["-nt", "-np", "-f"])
        .arg(&wav)
        .output()
        .await
        .with_context(|| format!("run {}", bin.display()))?;
    let _ = std::fs::remove_dir_all(&dir);
    anyhow::ensure!(out.status.success(), "speech to text failed: {}", String::from_utf8_lossy(&out.stderr));
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Text to speech on the appliance: MP3 of `text` in the configured voice.
pub async fn speak(cfg: &ReadAloud, text: &str) -> Result<Vec<u8>> {
    use tokio::io::AsyncWriteExt;
    let mut c = tokio::process::Command::new(&cfg.speak)
        .arg("--model")
        .arg(&cfg.model)
        .arg("--voices")
        .arg(&cfg.voices)
        .args(["--voice", &cfg.voice])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("run {}", cfg.speak.display()))?;
    let mut stdin = c.stdin.take().expect("piped");
    stdin.write_all(text.as_bytes()).await?;
    drop(stdin);
    let out = c.wait_with_output().await?;
    anyhow::ensure!(out.status.success() && !out.stdout.is_empty(), "read-aloud failed: {}", String::from_utf8_lossy(&out.stderr));
    Ok(out.stdout)
}
