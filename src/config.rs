use anyhow::{Context, Result};
use dirs::{config_dir, data_local_dir};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreset {
    Nord,
    Sunset,
    Forest,
}

impl ThemePreset {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "nord" => Self::Nord,
            "sunset" => Self::Sunset,
            "forest" => Self::Forest,
            _ => Self::Nord,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nord => "nord",
            Self::Sunset => "sunset",
            Self::Forest => "forest",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub linear_api_token: Option<String>,
    pub workspace_name: String,
    pub theme: Option<ThemePreset>,
}

impl WorkspaceConfig {
    pub fn load() -> Result<Self> {
        let data_dir = data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("logit");
        ensure_dir(&data_dir)?;

        let database_path = data_dir.join("logit.db");
        let linear_api_token = env::var("LINEAR_API_KEY").ok();
        let workspace_name =
            read_config_value("workspace_name").unwrap_or_else(|| "Personal Workspace".into());
        let theme = read_config_value("theme").map(|value| ThemePreset::parse(&value));

        Ok(Self {
            data_dir,
            database_path,
            linear_api_token,
            workspace_name,
            theme,
        })
    }
}

fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("creating data directory at {}", path.display()))
}

fn read_config_value(key: &str) -> Option<String> {
    let config_path = config_dir()?.join("logit").join("config.toml");
    let raw = fs::read_to_string(config_path).ok()?;

    raw.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || !trimmed.starts_with(key) {
            return None;
        }

        let (_, value) = trimmed.split_once('=')?;
        Some(value.trim().trim_matches('"').to_string())
    })
}
