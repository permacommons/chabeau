use crate::core::config::data::Config;
use std::fs;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

#[derive(Default)]
pub(crate) struct ConfigCacheState {
    config: Option<Config>,
    modified: Option<SystemTime>,
}

pub(crate) struct ConfigOrchestrator {
    path: PathBuf,
    state: Mutex<ConfigCacheState>,
}

pub(crate) static CONFIG_ORCHESTRATOR: LazyLock<ConfigOrchestrator> =
    LazyLock::new(|| ConfigOrchestrator::new(Config::get_config_path()));

#[cfg(test)]
pub(crate) static TEST_ORCHESTRATOR: LazyLock<Mutex<Option<ConfigOrchestrator>>> =
    LazyLock::new(|| Mutex::new(None));

impl ConfigOrchestrator {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(ConfigCacheState::default()),
        }
    }

    pub(crate) fn load_with_cache(&self) -> Result<Config, Box<dyn std::error::Error>> {
        let mut state = self.state.lock().unwrap();
        let disk_modified = Self::modified_time(&self.path);
        if state.config.is_none() || state.modified != disk_modified {
            let config = Config::load_from_path(&self.path)?;
            state.modified = disk_modified;
            state.config = Some(config);
        }
        Ok(state.config.clone().unwrap_or_default())
    }

    pub(crate) fn persist(&self, config: Config) -> Result<(), Box<dyn std::error::Error>> {
        self.write_config(&config)?;
        let mut state = self.state.lock().unwrap();
        state.modified = Self::modified_time(&self.path);
        state.config = Some(config);
        Ok(())
    }

    pub(crate) fn mutate<F, T>(&self, mutator: F) -> Result<T, Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut Config) -> Result<T, Box<dyn std::error::Error>>,
    {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            let disk_modified = Self::modified_time(&self.path);
            if state.config.is_none() || state.modified != disk_modified {
                let config = Config::load_from_path(&self.path)?;
                state.modified = disk_modified;
                state.config = Some(config);
            }
            state.config.clone().unwrap_or_default()
        };

        let mut working = snapshot;
        let result = mutator(&mut working)?;
        self.persist(working)?;
        Ok(result)
    }

    fn write_config(&self, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
        config.save_to_path(&self.path)
    }

    fn modified_time(path: &PathBuf) -> Option<SystemTime> {
        fs::metadata(path).ok()?.modified().ok()
    }
}

impl Config {
    pub fn load() -> Result<Config, Box<dyn std::error::Error>> {
        #[cfg(test)]
        {
            if let Some(orchestrator) = TEST_ORCHESTRATOR.lock().unwrap().as_ref() {
                return orchestrator.load_with_cache();
            }
        }
        CONFIG_ORCHESTRATOR.load_with_cache()
    }

    #[cfg(test)]
    pub fn load_test_safe() -> Result<Config, Box<dyn std::error::Error>> {
        Ok(Config::default())
    }

    #[cfg(not(test))]
    pub fn load_test_safe() -> Result<Config, Box<dyn std::error::Error>> {
        Self::load()
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        #[cfg(test)]
        {
            if let Some(orchestrator) = TEST_ORCHESTRATOR.lock().unwrap().as_ref() {
                return orchestrator.persist(self.clone());
            }
        }
        CONFIG_ORCHESTRATOR.persist(self.clone())
    }

    #[cfg(not(test))]
    pub fn mutate<F, T>(mutator: F) -> Result<T, Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut Config) -> Result<T, Box<dyn std::error::Error>>,
    {
        CONFIG_ORCHESTRATOR.mutate(mutator)
    }

    #[cfg(test)]
    pub fn mutate<F, T>(mutator: F) -> Result<T, Box<dyn std::error::Error>>
    where
        F: FnOnce(&mut Config) -> Result<T, Box<dyn std::error::Error>>,
    {
        if let Some(orchestrator) = TEST_ORCHESTRATOR.lock().unwrap().as_ref() {
            orchestrator.mutate(mutator)
        } else {
            let mut config = Config::default();
            let result = mutator(&mut config)?;
            Ok(result)
        }
    }

    #[cfg(test)]
    pub(crate) fn set_test_config_path(path: PathBuf) {
        let mut guard = TEST_ORCHESTRATOR.lock().unwrap();
        *guard = Some(ConfigOrchestrator::new(path));
    }

    #[cfg(test)]
    pub(crate) fn clear_test_config_override() {
        let mut guard = TEST_ORCHESTRATOR.lock().unwrap();
        guard.take();
    }
}
