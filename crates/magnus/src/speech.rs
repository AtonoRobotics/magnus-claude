//! Local speech to text with whisper.cpp, so voice never leaves the appliance.
//! Expects `whisper-cli` on PATH (or MAGNUS_WHISPER) and a model at
//! MAGNUS_WHISPER_MODEL; audio is converted with ffmpeg.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use std::path::Path;

pub async fn transcribe(state_dir: &Path, data_b64: &str) -> Result<String> {
    let bin = std::env::var("MAGNUS_WHISPER").unwrap_or_else(|_| "whisper-cli".into());
    let model = std::env::var("MAGNUS_WHISPER_MODEL").map_err(|_| anyhow!("no speech-to-text model is installed on this appliance"))?;
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
        .args(["-m", &model, "-nt", "-np", "-f"])
        .arg(&wav)
        .output()
        .await
        .with_context(|| format!("run {bin}"))?;
    let _ = std::fs::remove_dir_all(&dir);
    anyhow::ensure!(out.status.success(), "speech to text failed: {}", String::from_utf8_lossy(&out.stderr));
    Ok(String::from_utf8_lossy(&out.stdout).split_whitespace().collect::<Vec<_>>().join(" "))
}
