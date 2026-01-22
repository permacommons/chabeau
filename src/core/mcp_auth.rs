use crate::core::keyring::KeyringAccessError;
use keyring::Entry;

const KEYRING_SERVICE: &str = "chabeau-mcp";

#[derive(Debug, Clone, Copy)]
pub struct McpTokenStore {
    use_keyring: bool,
}

impl Default for McpTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl McpTokenStore {
    pub fn new() -> Self {
        Self { use_keyring: true }
    }

    pub fn new_with_keyring(use_keyring: bool) -> Self {
        Self { use_keyring }
    }

    pub fn get_token(&self, server_id: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if !self.use_keyring {
            return Ok(None);
        }

        let entry = Entry::new(KEYRING_SERVICE, server_id)?;
        match entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(Box::new(KeyringAccessError::from(err))),
        }
    }

    pub fn set_token(
        &self,
        server_id: &str,
        token: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.use_keyring {
            return Ok(());
        }

        let entry = Entry::new(KEYRING_SERVICE, server_id)?;
        entry
            .set_password(token)
            .map_err(|err| Box::new(KeyringAccessError::from(err)) as Box<dyn std::error::Error>)
    }

    pub fn remove_token(&self, server_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
        if !self.use_keyring {
            return Ok(false);
        }

        let entry = Entry::new(KEYRING_SERVICE, server_id)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(err) => Err(Box::new(KeyringAccessError::from(err))),
        }
    }
}
