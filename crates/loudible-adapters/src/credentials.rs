use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use loudible_core::{AudibleSession, CredentialStore};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

/// Holds one secret text in a place the OS protects. Errors are technical descriptions.
pub trait SecretStore: Send + Sync + 'static {
    fn read(&self) -> Result<Option<String>, String>;
    fn write(&self, secret: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// macOS Keychain, Windows Credential Manager, or Secret Service on Linux.
pub struct KeyringSecretStore {
    service: String,
    account: String,
}

impl KeyringSecretStore {
    pub const DEFAULT_SERVICE: &str = "Loudible.SessionKey";

    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, &self.account).map_err(|error| error.to_string())
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new(Self::DEFAULT_SERVICE, "default")
    }
}

impl SecretStore for KeyringSecretStore {
    fn read(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write(&self, secret: &str) -> Result<(), String> {
        self.entry()?
            .set_password(secret)
            .map_err(|error| error.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Bounds a store that can block forever, as Secret Service does on a locked keyring. The call
/// is abandoned on its thread, and after one hang the store fails at once instead of piling up
/// more stuck threads.
pub struct TimeoutSecretStore<S> {
    inner: Arc<S>,
    limit: Duration,
    hung: Arc<AtomicBool>,
}

impl<S: SecretStore> TimeoutSecretStore<S> {
    pub fn new(inner: S, limit: Duration) -> Self {
        Self {
            inner: Arc::new(inner),
            limit,
            hung: Arc::new(AtomicBool::new(false)),
        }
    }

    fn call<T: Send + 'static>(
        &self,
        body: impl FnOnce(&S) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        const UNAVAILABLE: &str = "The system's secret store is not responding.";
        if self.hung.load(Ordering::Relaxed) {
            return Err(UNAVAILABLE.to_string());
        }
        let (sender, receiver) = mpsc::channel();
        let inner = self.inner.clone();
        std::thread::spawn(move || {
            let _ = sender.send(body(&inner));
        });
        receiver.recv_timeout(self.limit).unwrap_or_else(|_| {
            self.hung.store(true, Ordering::Relaxed);
            Err(UNAVAILABLE.to_string())
        })
    }
}

impl<S: SecretStore> SecretStore for TimeoutSecretStore<S> {
    fn read(&self) -> Result<Option<String>, String> {
        self.call(S::read)
    }

    fn write(&self, secret: &str) -> Result<(), String> {
        let secret = secret.to_string();
        self.call(move |store| store.write(&secret))
    }

    fn delete(&self) -> Result<(), String> {
        self.call(S::delete)
    }
}

/// An owner-only key file for systems with no usable secret store. The file sits unencrypted on
/// disk, so it is used only once the user has opted in by creating it with `opt_in`.
pub struct KeyFileSecretStore {
    path: PathBuf,
}

impl KeyFileSecretStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Creates the empty file whose presence selects this store.
    pub fn opt_in(&self) -> Result<(), String> {
        if self.path.exists() {
            return Ok(());
        }
        write_privately(&self.path, b"").map_err(|error| error.to_string())
    }
}

impl SecretStore for KeyFileSecretStore {
    fn read(&self) -> Result<Option<String>, String> {
        match fs::read_to_string(&self.path) {
            Ok(secret) => Ok(Some(secret).filter(|secret| !secret.is_empty())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write(&self, secret: &str) -> Result<(), String> {
        write_privately(&self.path, secret.as_bytes()).map_err(|error| error.to_string())
    }

    // Emptied rather than removed, so the user's opt-in outlives signing out.
    fn delete(&self) -> Result<(), String> {
        if !self.path.exists() {
            return Ok(());
        }
        write_privately(&self.path, b"").map_err(|error| error.to_string())
    }
}

/// Writes `contents` with mode 0600, atomically, creating the folder if needed.
fn write_privately(path: &Path, contents: &[u8]) -> io::Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| io::Error::other("the path has no folder"))?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(folder)?;
    // Temporary files are created owner-only.
    let mut file = tempfile::Builder::new()
        .prefix(".credentials")
        .tempfile_in(folder)?;
    file.write_all(contents)?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Stores the session as AES-256-GCM encrypted JSON in a file, with only the random key in the
/// secret store: Windows Credential Manager cannot hold a whole session.
pub struct EncryptedFileCredentialStore {
    file: PathBuf,
    secrets: Arc<dyn SecretStore>,
}

const FILE_MAGIC: &[u8] = b"LOUDIBLE-SESSION-1";
const KEY_LENGTH: usize = 32;

impl EncryptedFileCredentialStore {
    pub fn new(file: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        Self { file, secrets }
    }

    fn cipher(key: &[u8]) -> Result<LessSafeKey, String> {
        UnboundKey::new(&AES_256_GCM, key)
            .map(LessSafeKey::new)
            .map_err(|_| "The saved key has the wrong length.".to_string())
    }

    fn encryption_key(&self) -> Result<Vec<u8>, String> {
        if let Some(secret) = self.secrets.read()? {
            return Self::decode_key(&secret);
        }
        let mut key = vec![0; KEY_LENGTH];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|error| error.to_string())?;
        self.secrets.write(&STANDARD.encode(&key))?;
        Ok(key)
    }

    fn decode_key(secret: &str) -> Result<Vec<u8>, String> {
        STANDARD
            .decode(secret.trim())
            .map_err(|error| format!("The saved key is unreadable: {error}"))
    }

    fn decrypt(key: &[u8], contents: &[u8]) -> Result<Vec<u8>, String> {
        let unreadable = || "the file is damaged or was written with another key".to_string();
        let body = contents.strip_prefix(FILE_MAGIC).ok_or_else(unreadable)?;
        if body.len() < NONCE_LEN {
            return Err(unreadable());
        }
        let (nonce, ciphertext) = body.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| unreadable())?;
        let mut buffer = ciphertext.to_vec();
        let plaintext = Self::cipher(key)?
            .open_in_place(nonce, Aad::from(FILE_MAGIC), &mut buffer)
            .map_err(|_| unreadable())?;
        Ok(plaintext.to_vec())
    }
}

impl CredentialStore for EncryptedFileCredentialStore {
    /// Without a session file the secret store is left alone, so a first launch prompts for nothing.
    fn load(&self) -> Result<Option<AudibleSession>, String> {
        let contents = match fs::read(&self.file) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Could not read saved credentials: {error}")),
        };
        let unreadable = |detail| format!("Saved credentials are unreadable: {detail}");
        let secret = self
            .secrets
            .read()?
            .ok_or_else(|| unreadable("the key is missing".to_string()))?;
        let key = Self::decode_key(&secret)?;
        let plaintext = Self::decrypt(&key, &contents).map_err(unreadable)?;
        serde_json::from_slice(&plaintext)
            .map(Some)
            .map_err(|error| unreadable(error.to_string()))
    }

    fn save(&self, session: &AudibleSession) -> Result<(), String> {
        let mut buffer = serde_json::to_vec(session).map_err(|error| error.to_string())?;
        let key = self.encryption_key()?;
        let mut nonce = [0; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|error| error.to_string())?;
        Self::cipher(&key)?
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(FILE_MAGIC),
                &mut buffer,
            )
            .map_err(|error| error.to_string())?;
        let contents = [FILE_MAGIC, &nonce, &buffer].concat();
        write_privately(&self.file, &contents)
            .map_err(|error| format!("Could not save credentials: {error}"))
    }

    /// Removes the file first: signing out must hold even if the key cannot be deleted.
    fn delete(&self) -> Result<(), String> {
        match fs::remove_file(&self.file) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not delete credentials: {error}")),
        }
        self.secrets.delete()
    }
}

/// How long Secret Service gets to answer before it counts as unavailable.
pub const SECRET_SERVICE_LIMIT: Duration = Duration::from_secs(5);

/// The session in `session_file`, keyed from the OS store unless `key_file` has been opted into.
pub fn default_credential_store(
    session_file: PathBuf,
    key_file: PathBuf,
) -> EncryptedFileCredentialStore {
    let secrets: Arc<dyn SecretStore> = if key_file.exists() {
        Arc::new(KeyFileSecretStore::new(key_file))
    } else if cfg!(target_os = "linux") {
        Arc::new(TimeoutSecretStore::new(
            KeyringSecretStore::default(),
            SECRET_SERVICE_LIMIT,
        ))
    } else {
        // A macOS Keychain prompt legitimately waits for the user, so it has no limit.
        Arc::new(KeyringSecretStore::default())
    };
    EncryptedFileCredentialStore::new(session_file, secrets)
}
