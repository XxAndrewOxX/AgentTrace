use crate::config::GlobalConfig;
use crate::ModelCmd;
use anyhow::Result;

pub fn run(cmd: ModelCmd) -> Result<()> {
    match cmd {
        ModelCmd::Info => {
            let config = GlobalConfig::load()?;
            match &config.llm.model_path {
                Some(path) => println!("Model: {}", path.display()),
                None => println!("No model configured. Use `docmgr model set <path>` or `docmgr model download`."),
            }
        }
        ModelCmd::Set { path } => {
            if !path.exists() {
                anyhow::bail!("Model file not found: {}", path.display());
            }
            let mut config = GlobalConfig::load()?;
            config.llm.model_path = Some(path.clone());
            config.save()?;
            println!("Model set to {}", path.display());
        }
        ModelCmd::Download { size } => {
            println!("Model download not yet implemented (size: {}).", size);
            println!("Visit https://huggingface.co to download a GGUF model manually.");
        }
    }
    Ok(())
}
