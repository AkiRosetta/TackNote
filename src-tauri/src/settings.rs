//! The WebDAV account. The address and user name live in an ordinary settings
//! file; the password lives only in the system keychain and is never written
//! anywhere else, not even when the keychain fails.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::webdav::WebDavConfig;

/// Where the password is kept.
pub trait SecretStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, password: &str) -> Result<(), String>;
}

/// The system keychain on macOS and Windows.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct Keychain {
    entry: keyring::Entry,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl Keychain {
    pub fn new(service: &str) -> Result<Keychain, String> {
        let entry = keyring::Entry::new(service, "webdav").map_err(|error| error.to_string())?;
        Ok(Keychain { entry })
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl SecretStore for Keychain {
    fn get(&self) -> Result<Option<String>, String> {
        match self.entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn set(&self, password: &str) -> Result<(), String> {
        self.entry.set_password(password).map_err(|error| error.to_string())
    }
}

/// For platforms without secure storage yet. Saving fails loudly rather than
/// falling back to a plain file.
pub struct NoSecureStorage;

impl SecretStore for NoSecureStorage {
    fn get(&self) -> Result<Option<String>, String> {
        Err("this platform has no secure password storage yet".into())
    }

    fn set(&self, _password: &str) -> Result<(), String> {
        Err("this platform has no secure password storage yet".into())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Account {
    url: String,
    username: String,
}

#[derive(Debug)]
pub enum SettingsError {
    /// The settings file could not be written.
    Io(io::Error),
    /// The keychain refused the password; nothing was saved in its place.
    Keychain(String),
}

pub struct Settings {
    path: PathBuf,
    secret: Box<dyn SecretStore>,
}

impl Settings {
    pub fn new(path: impl Into<PathBuf>, secret: Box<dyn SecretStore>) -> Settings {
        Settings { path: path.into(), secret }
    }

    /// The saved account without its password, which needs the keychain.
    pub fn account(&self) -> WebDavConfig {
        let account: Account = fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        WebDavConfig { url: account.url, username: account.username, password: String::new() }
    }

    /// Without an account there is no password to read, so a keychain problem
    /// never shows up as a sync error on a fresh install.
    pub fn load_webdav(&self) -> Result<WebDavConfig, String> {
        let account = self.account();
        if !account.is_complete() {
            return Ok(account);
        }
        let password = self.secret.get()?.unwrap_or_default();
        Ok(WebDavConfig { password, ..account })
    }

    pub fn save_webdav(&self, config: &WebDavConfig) -> Result<(), SettingsError> {
        self.secret.set(&config.password).map_err(SettingsError::Keychain)?;
        let account = Account { url: config.url.trim().to_string(), username: config.username.trim().to_string() };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(SettingsError::Io)?;
        }
        let json = serde_json::to_vec_pretty(&account).map_err(|error| SettingsError::Io(error.into()))?;
        fs::write(&self.path, json).map_err(SettingsError::Io)
    }
}

#[cfg(test)]
pub mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::model::new_id;

    /// An in-memory keychain that counts reads, and can be made to fail.
    #[derive(Clone, Default)]
    pub struct FakeSecret {
        pub value: Arc<Mutex<Option<String>>>,
        pub reads: Arc<AtomicUsize>,
        pub broken: bool,
    }

    impl SecretStore for FakeSecret {
        fn get(&self) -> Result<Option<String>, String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.broken {
                return Err("keychain locked".into());
            }
            Ok(self.value.lock().unwrap().clone())
        }

        fn set(&self, password: &str) -> Result<(), String> {
            if self.broken {
                return Err("keychain locked".into());
            }
            *self.value.lock().unwrap() = Some(password.into());
            Ok(())
        }
    }

    pub fn temp_settings_path() -> PathBuf {
        std::env::temp_dir().join(format!("tacknote-settings-{}", new_id())).join("settings.json")
    }

    fn config() -> WebDavConfig {
        WebDavConfig { url: " https://example.test/dav ".into(), username: "me".into(), password: "secret".into() }
    }

    #[test]
    fn the_password_goes_to_the_keychain_and_never_to_the_file() {
        let path = temp_settings_path();
        let secret = FakeSecret::default();
        let settings = Settings::new(&path, Box::new(secret.clone()));
        settings.save_webdav(&config()).unwrap();

        assert!(!fs::read_to_string(&path).unwrap().contains("secret"));
        assert_eq!(secret.value.lock().unwrap().as_deref(), Some("secret"));
        let loaded = settings.load_webdav().unwrap();
        assert_eq!(loaded.url, "https://example.test/dav");
        assert_eq!(loaded.password, "secret");
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_keychain_failure_saves_nothing_in_plain_text() {
        let path = temp_settings_path();
        let settings = Settings::new(&path, Box::new(FakeSecret { broken: true, ..FakeSecret::default() }));
        assert!(matches!(settings.save_webdav(&config()), Err(SettingsError::Keychain(_))));
        assert!(!path.exists());
    }

    #[test]
    fn without_an_account_the_keychain_is_never_read() {
        let secret = FakeSecret { broken: true, ..FakeSecret::default() };
        let settings = Settings::new(temp_settings_path(), Box::new(secret.clone()));
        assert!(!settings.load_webdav().unwrap().is_complete());
        assert_eq!(secret.reads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn platforms_without_secure_storage_refuse_to_save() {
        let path = temp_settings_path();
        let settings = Settings::new(&path, Box::new(NoSecureStorage));
        assert!(matches!(settings.save_webdav(&config()), Err(SettingsError::Keychain(_))));
        assert!(!path.exists());
    }
}
