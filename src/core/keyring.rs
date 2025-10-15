use std::error::Error;
use std::fmt;

/// Describes failures when attempting to access the system keyring.
///
/// Recoverable errors indicate that the credential backend was
/// temporarily unavailable (for example when the keychain service is
/// locked or inaccessible). Permanent errors surface the underlying
/// cause directly so callers can report them to the user.
#[derive(Debug)]
pub enum KeyringAccessError {
    Recoverable(keyring::Error),
    Permanent(keyring::Error),
}

impl KeyringAccessError {
    fn inner(&self) -> &keyring::Error {
        match self {
            KeyringAccessError::Recoverable(err) | KeyringAccessError::Permanent(err) => err,
        }
    }

    /// Returns true when the error represents a temporary outage of the
    /// platform keyring backend.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, KeyringAccessError::Recoverable(_))
    }
}

impl From<keyring::Error> for KeyringAccessError {
    fn from(err: keyring::Error) -> Self {
        match err {
            keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_) => {
                KeyringAccessError::Recoverable(err)
            }
            other => KeyringAccessError::Permanent(other),
        }
    }
}

impl fmt::Display for KeyringAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner())
    }
}

impl Error for KeyringAccessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.inner())
    }
}
