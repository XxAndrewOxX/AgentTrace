use crate::config::GlobalConfig;
use crate::observability::CliOutput;
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use indicatif::{ProgressBar, ProgressStyle};
use std::io::Write;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum ModelCmd {
    /// Download a model from Hugging Face.
    Download {
        /// Model size variant.
        #[arg(long, default_value = "3b")]
        size: String,
    },
    /// Show information about the current model.
    Info,
    /// Set the model to use.
    Set {
        /// Path to the GGUF model file.
        path: PathBuf,
    },
}

/// Known model variants: (size_key, hf_repo, filename)
const MODELS: &[(&str, &str, &str)] = &[
    (
        "3b",
        "bartowski/Llama-3.2-3B-Instruct-GGUF",
        "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
    ),
    (
        "7b",
        "bartowski/Mistral-7B-Instruct-v0.3-GGUF",
        "Mistral-7B-Instruct-v0.3-Q4_K_M.gguf",
    ),
    (
        "1b",
        "bartowski/Llama-3.2-1B-Instruct-GGUF",
        "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
    ),
];

pub fn run(cmd: ModelCmd, output: &dyn CliOutput) -> Result<()> {
    match cmd {
        ModelCmd::Info => {
            let config = GlobalConfig::load()?;
            match &config.llm.model_path {
                Some(path) => {
                    if path.exists() {
                        let meta = std::fs::metadata(path)?;
                        output.line(&format!("Model: {}", path.display()))?;
                        output.line(&format!(
                            "Size:  {:.1} GB",
                            meta.len() as f64 / 1_073_741_824.0
                        ))?;
                    } else {
                        output.line(&format!(
                            "Model configured but file not found: {}",
                            path.display()
                        ))?;
                    }
                }
                None => output.line(
                    "No model configured.\n\
                     Use `agent-trace model download` or `agent-trace model set <path>`.",
                )?,
            }
        }

        ModelCmd::Set { path } => {
            if !path.exists() {
                bail!("Model file not found: {}", path.display());
            }
            let abs = path.canonicalize()?;
            let mut config = GlobalConfig::load()?;
            config.llm.model_path = Some(abs.clone());
            config.save()?;
            output.line(&format!("Model set to {}", abs.display()))?;
        }

        ModelCmd::Download { size } => {
            let (_, repo, filename) = MODELS
                .iter()
                .find(|(s, _, _)| *s == size.as_str())
                .ok_or_else(|| {
                    let valid: Vec<_> = MODELS.iter().map(|(s, _, _)| *s).collect();
                    anyhow::anyhow!(
                        "Unknown model size '{}'. Valid options: {}",
                        size,
                        valid.join(", ")
                    )
                })?;

            let dest_dir = model_dir()?;
            std::fs::create_dir_all(&dest_dir)?;
            let dest_path = dest_dir.join(filename);

            if dest_path.exists() {
                output.line(&format!(
                    "Model already downloaded: {}",
                    dest_path.display()
                ))?;
                let mut config = GlobalConfig::load()?;
                config.llm.model_path = Some(dest_path);
                config.save()?;
                return Ok(());
            }

            let url = format!("https://huggingface.co/{}/resolve/main/{}", repo, filename);

            output.line(&format!("Downloading {} ({} model)…", filename, size))?;
            output.line(&format!("Source: {}", url))?;

            download_with_progress(&url, &dest_path)?;

            let mut config = GlobalConfig::load()?;
            config.llm.model_path = Some(dest_path.clone());
            config.save()?;

            output.line(&format!("Saved to {}", dest_path.display()))?;
            output.line("Global config updated — LLM will be used on next `agent-trace open`.")?;
        }
    }
    Ok(())
}

fn model_dir() -> Result<PathBuf> {
    let base = dirs_next::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    Ok(base.join("agent-trace").join("models"))
}

fn download_with_progress(url: &str, dest: &std::path::Path) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    let resp = client
        .get(url)
        .send()
        .with_context(|| format!("GET {}", url))?;

    if !resp.status().is_success() {
        bail!("Download failed: HTTP {}", resp.status());
    }

    let total = resp.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        )
        .unwrap()
        .progress_chars("#>-"),
    );

    // Write to a temp file, then rename atomically.
    let tmp = dest.with_extension("gguf.tmp");
    let mut file =
        std::fs::File::create(&tmp).with_context(|| format!("Creating {}", tmp.display()))?;

    let mut downloaded: u64 = 0;
    let mut buf = [0u8; 65536];
    let mut reader = resp;

    use std::io::Read;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("download complete");
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_model_sizes() {
        for size in ["1b", "3b", "7b"] {
            assert!(
                MODELS.iter().any(|(s, _, _)| *s == size),
                "Missing model size: {}",
                size
            );
        }
    }

    #[test]
    fn test_unknown_size_error() {
        let result = MODELS.iter().find(|(s, _, _)| *s == "99b");
        assert!(result.is_none());
    }

    #[test]
    fn test_model_dir_is_absolute() {
        let dir = model_dir().unwrap();
        assert!(dir.is_absolute());
        assert!(dir.to_string_lossy().contains("agent-trace"));
    }
}
