use crate::core::config::data::{path_display, Config};
use directories::ProjectDirs;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// Errors that can occur when loading configuration from disk.
#[derive(Debug)]
pub enum ConfigError {
    /// Failed to read the configuration file from disk.
    Read {
        /// Path to the configuration file that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },

    /// Failed to parse the configuration file as valid TOML.
    Parse {
        /// Path to the configuration file with invalid TOML.
        path: PathBuf,
        /// The TOML deserialization error.
        source: toml::de::Error,
    },
}

impl ConfigError {
    fn display_path(path: &Path) -> String {
        path_display(path)
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => {
                write!(
                    f,
                    "Failed to read config at {}: {}",
                    Self::display_path(path),
                    source
                )
            }
            ConfigError::Parse { path, source } => {
                write!(
                    f,
                    "Failed to parse config at {}: {}",
                    Self::display_path(path),
                    source
                )
            }
        }
    }
}

impl StdError for ConfigError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            ConfigError::Read { source, .. } => Some(source),
            ConfigError::Parse { source, .. } => Some(source),
        }
    }
}

impl Config {
    pub fn load_from_path(config_path: &PathBuf) -> Result<Config, Box<dyn std::error::Error>> {
        if config_path.exists() {
            let contents = fs::read_to_string(config_path).map_err(|source| ConfigError::Read {
                path: config_path.clone(),
                source,
            })?;
            let config: Config =
                toml::from_str(&contents).map_err(|source| ConfigError::Parse {
                    path: config_path.clone(),
                    source,
                })?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub(crate) fn save_to_path(
        &self,
        config_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let parent = config_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty());

        if let Some(dir) = parent {
            fs::create_dir_all(dir)?;
        }

        let contents = toml::to_string_pretty(self)?;
        let mut temp_file = match parent {
            Some(dir) => NamedTempFile::new_in(dir)?,
            None => NamedTempFile::new()?,
        };

        temp_file.write_all(contents.as_bytes())?;
        temp_file.as_file_mut().sync_all()?;
        temp_file
            .persist(config_path)
            .map_err(|err| -> Box<dyn std::error::Error> { Box::new(err) })?;
        Ok(())
    }

    pub(crate) fn get_config_path() -> PathBuf {
        let proj_dirs = ProjectDirs::from("org", "permacommons", "chabeau")
            .expect("Failed to determine config directory");
        proj_dirs.config_dir().join("config.toml")
    }

    #[cfg(test)]
    pub(crate) fn test_config_path() -> PathBuf {
        Self::get_config_path()
    }
}
