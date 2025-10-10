#[cfg(test)]
use crate::core::app::App;
#[cfg(test)]
use crate::core::config::Config;
#[cfg(test)]
use crate::core::message::Message;
#[cfg(test)]
use crate::ui::theme::Theme;
#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(test)]
use std::collections::VecDeque;
#[cfg(test)]
use std::env;
#[cfg(test)]
use std::ffi::{OsStr, OsString};
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard};
#[cfg(test)]
use tempfile::TempDir;

#[cfg(test)]
static TEST_CONFIG_ENV_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[cfg(test)]
static TEST_ENV_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[cfg(test)]
pub struct TestConfigEnv {
    _lock: MutexGuard<'static, ()>,
    temp_dir: TempDir,
    previous_vars: Vec<(String, Option<OsString>)>,
}

#[cfg(test)]
impl TestConfigEnv {
    pub fn new() -> Self {
        let lock = TEST_CONFIG_ENV_GUARD
            .lock()
            .expect("config env mutex poisoned");
        let temp_dir = TempDir::new().expect("failed to create temp dir for config");
        let mut guard = Self {
            _lock: lock,
            temp_dir,
            previous_vars: Vec::new(),
        };

        guard.capture_and_set("XDG_CONFIG_HOME");

        #[cfg(target_os = "windows")]
        {
            guard.capture_and_set("APPDATA");
            guard.capture_and_set("LOCALAPPDATA");
        }

        #[cfg(target_os = "macos")]
        {
            guard.capture_and_set("HOME");
        }

        Config::set_test_config_path(Config::test_config_path());

        guard
    }

    fn capture_and_set(&mut self, key: &str) {
        let previous = env::var_os(key);
        self.previous_vars.push((key.to_string(), previous));
        env::set_var(key, self.temp_dir.path());
    }

    pub fn config_root(&self) -> &Path {
        self.temp_dir.path()
    }
}

#[cfg(test)]
impl Drop for TestConfigEnv {
    fn drop(&mut self) {
        Config::clear_test_config_override();
        for (key, value) in self.previous_vars.drain(..).rev() {
            if let Some(val) = value {
                env::set_var(&key, val);
            } else {
                env::remove_var(&key);
            }
        }
    }
}

#[cfg(test)]
impl Default for TestConfigEnv {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub fn with_test_config_env<F, T>(f: F) -> T
where
    F: FnOnce(&Path) -> T,
{
    let guard = TestConfigEnv::new();
    let result = f(guard.config_root());
    result
}

#[cfg(test)]
pub struct TestEnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    previous_vars: Vec<(OsString, Option<OsString>)>,
}

#[cfg(test)]
impl TestEnvVarGuard {
    pub fn new() -> Self {
        let lock = TEST_ENV_GUARD.lock().expect("environment mutex poisoned");
        Self {
            _lock: lock,
            previous_vars: Vec::new(),
        }
    }

    fn capture_if_needed(&mut self, key: &OsStr) {
        if self
            .previous_vars
            .iter()
            .any(|(existing, _)| existing.as_os_str() == key)
        {
            return;
        }
        let previous = env::var_os(key);
        self.previous_vars.push((key.to_os_string(), previous));
    }

    pub fn set_var<K, V>(&mut self, key: K, value: V)
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let key_ref = key.as_ref();
        self.capture_if_needed(key_ref);
        env::set_var(key_ref, value);
    }

    pub fn remove_var<K>(&mut self, key: K)
    where
        K: AsRef<OsStr>,
    {
        let key_ref = key.as_ref();
        self.capture_if_needed(key_ref);
        env::remove_var(key_ref);
    }
}

#[cfg(test)]
impl Drop for TestEnvVarGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous_vars.drain(..).rev() {
            if let Some(val) = value {
                env::set_var(&key, val);
            } else {
                env::remove_var(&key);
            }
        }
    }
}

#[cfg(test)]
impl Default for TestEnvVarGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
pub fn create_test_app() -> App {
    let mut app = App::new_test_app(Theme::dark_default(), true, true);
    app.session.model = "test-model".to_string();
    app.session.api_key = "test-key".to_string();
    app.session.base_url = "https://api.test.com".to_string();
    app.session.provider_name = "test".to_string();
    app.session.provider_display_name = "Test".to_string();
    app
}

#[cfg(test)]
pub fn create_test_message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_string(),
        content: content.to_string(),
    }
}

#[cfg(test)]
pub fn create_test_messages() -> VecDeque<Message> {
    let mut messages = VecDeque::new();
    messages.push_back(create_test_message("user", "Hello"));
    messages.push_back(create_test_message("assistant", "Hi there!"));
    messages.push_back(create_test_message("user", "How are you?"));
    messages.push_back(create_test_message(
        "assistant",
        "I'm doing well, thank you for asking!",
    ));
    messages
}

#[cfg(test)]
pub const SAMPLE_HYPERTEXT_PARAGRAPH: &str = "The story of hypertext begins not with Tim Berners-Lee's World Wide Web, but with Vannevar Bush's 1945 essay \"As We May Think,\" where he envisioned the Memex - a device that would store books, records, and communications, and mechanically link them together by association. Ted Nelson, inspired by Bush's vision, coined the term \"hypertext\" in 1963 and spent decades developing [the original web proposal](https://www.example.com) - a system that would revolutionize how we think about documents, copyright, and knowledge itself. Nelson's Xanadu wasn't just about linking documents; it was about creating a [hypertext dreams](https://docs.hypertext.org) where every quotation would be automatically linked to its source, authors would be compensated for every use of their work, and the sum of human knowledge would be accessible through an elegant web of associations.";
