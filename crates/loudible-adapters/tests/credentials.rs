//! The credential store. An in-memory secret store keeps these off the real
//! Keychain.

use std::sync::Arc;
use std::time::{Duration, Instant};

use loudible_adapters::*;
use loudible_core::*;
use loudible_test_support::*;

#[derive(Default)]
struct FakeSecrets {
    secret: Locked<Option<String>>,
    fail_read: Locked<bool>,
    fail_write: Locked<bool>,
    fail_delete: Locked<bool>,
    reads: Locked<usize>,
}

impl SecretStore for FakeSecrets {
    fn read(&self) -> Result<Option<String>, String> {
        self.reads.with(|count| *count += 1);
        if self.fail_read.current() {
            return Err("read failure".into());
        }
        Ok(self.secret.current())
    }

    fn write(&self, secret: &str) -> Result<(), String> {
        if self.fail_write.current() {
            return Err("write failure".into());
        }
        self.secret.set(Some(secret.to_string()));
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        if self.fail_delete.current() {
            return Err("delete failure".into());
        }
        self.secret.set(None);
        Ok(())
    }
}

struct Fixture {
    directory: TemporaryDirectory,
    secrets: Arc<FakeSecrets>,
    store: EncryptedFileCredentialStore,
}

fn fixture() -> Fixture {
    let directory = TemporaryDirectory::default();
    let secrets = Arc::new(FakeSecrets::default());
    let store = EncryptedFileCredentialStore::new(directory.join("session.bin"), secrets.clone());
    Fixture {
        directory,
        secrets,
        store,
    }
}

#[test]
fn saves_loads_and_deletes() {
    let f = fixture();
    assert_eq!(f.store.load(), Ok(None));
    f.store.save(&session()).unwrap();
    assert_eq!(f.store.load(), Ok(Some(session())));

    let updated = session_with_token("new");
    f.store.save(&updated).unwrap();
    assert_eq!(f.store.load(), Ok(Some(updated)));
    assert_eq!(f.directory.contents(), ["session.bin"]);

    f.store.delete().unwrap();
    assert_eq!(f.store.load(), Ok(None));
    assert_eq!(f.secrets.secret.current(), None);
    f.store.delete().unwrap(); // deleting nothing is fine
}

#[test]
fn nothing_readable_reaches_the_disk_and_the_secret_is_small() {
    let f = fixture();
    f.store.save(&session()).unwrap();
    let contents = std::fs::read(f.directory.join("session.bin")).unwrap();
    let text = String::from_utf8_lossy(&contents);
    assert!(!text.contains("synthetic"));
    assert!(!text.contains("customer_id"));
    // Windows Credential Manager holds at most 2,560 bytes.
    assert!(f.secrets.secret.current().unwrap().len() < 100);
}

#[cfg(unix)]
#[test]
fn the_session_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    f.store.save(&session()).unwrap();
    let mode = std::fs::metadata(f.directory.join("session.bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_first_launch_does_not_touch_the_secret_store() {
    let f = fixture();
    f.store.load().unwrap();
    f.store.delete().unwrap();
    assert_eq!(f.secrets.reads.current(), 0);
}

#[test]
fn surfaces_secret_store_errors() {
    let f = fixture();
    f.secrets.fail_write.set(true);
    assert!(f.store.save(&session()).is_err());
    f.secrets.fail_write.set(false);
    f.store.save(&session()).unwrap();
    f.secrets.fail_read.set(true);
    assert_eq!(f.store.load(), Err("read failure".to_string()));
    assert!(f.store.save(&session()).is_err());
    f.secrets.fail_delete.set(true);
    assert!(f.store.delete().is_err());
    assert!(!f.directory.join("session.bin").exists());
}

#[test]
fn reports_unreadable_sessions() {
    let f = fixture();
    f.store.save(&session()).unwrap();
    let path = f.directory.join("session.bin");

    let original = std::fs::read(&path).unwrap();
    let mut damaged = original.clone();
    *damaged.last_mut().unwrap() ^= 1;
    std::fs::write(&path, damaged).unwrap();
    assert!(
        f.store
            .load()
            .unwrap_err()
            .starts_with("Saved credentials are unreadable")
    );

    std::fs::write(&path, b"not encrypted").unwrap();
    assert!(
        f.store
            .load()
            .unwrap_err()
            .starts_with("Saved credentials are unreadable")
    );

    std::fs::write(&path, &original).unwrap();
    f.secrets.secret.set(None);
    assert!(f.store.load().unwrap_err().contains("the key is missing"));
}

struct Blocking;

impl SecretStore for Blocking {
    fn read(&self) -> Result<Option<String>, String> {
        std::thread::sleep(Duration::from_secs(30));
        Ok(None)
    }

    fn write(&self, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_hanging_secret_store_times_out_and_then_fails_at_once() {
    let store = TimeoutSecretStore::new(Blocking, Duration::from_millis(20));
    let started = Instant::now();
    assert!(store.read().is_err());
    assert!(store.write("x").is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_responsive_secret_store_passes_through_the_timeout() {
    let store = TimeoutSecretStore::new(FakeSecrets::default(), Duration::from_secs(5));
    assert_eq!(store.read(), Ok(None));
    store.write("value").unwrap();
    assert_eq!(store.read(), Ok(Some("value".to_string())));
    store.delete().unwrap();
    assert_eq!(store.read(), Ok(None));
}

#[test]
fn key_file_is_chosen_only_after_opting_in() {
    let directory = TemporaryDirectory::default();
    let key_file = directory.join("folder/master.key");
    let keys = KeyFileSecretStore::new(key_file.clone());
    assert_eq!(keys.read(), Ok(None));
    assert!(!key_file.exists());
    keys.opt_in().unwrap();
    assert_eq!(keys.read(), Ok(None));

    // With the file present this never reaches the OS store.
    let store = default_credential_store(directory.join("session.bin"), key_file.clone());
    store.save(&session()).unwrap();
    assert_eq!(store.load(), Ok(Some(session())));
    assert!(keys.read().unwrap().is_some());

    store.delete().unwrap();
    assert_eq!(keys.read(), Ok(None));
    assert!(key_file.exists());
}
