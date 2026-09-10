use base64::Engine;
use keyring::Entry;
use rand::RngCore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};

const SERVICE_NAME: &str = "termdrop";

/// Where one secret lives, in both stores.
///
/// The keyring account and the fallback-file key are deliberately allowed to
/// differ: SSH passwords have always used `host-<id>` in the keyring but the
/// bare `<id>` in the fallback file, and changing either would orphan secrets
/// already on disk.
#[derive(Clone, Debug)]
pub struct Account {
    keyring: String,
    fallback: String,
}

impl Account {
    /// The SSH password for a host. Preserves the historical key shapes.
    fn host(host_id: i64) -> Self {
        Self {
            keyring: format!("host-{}", host_id),
            fallback: host_id.to_string(),
        }
    }

    /// Any other secret, keyed by an explicit name. Names are non-numeric, so
    /// they cannot collide with a host id in the shared fallback file.
    fn named(name: &str) -> Self {
        Self {
            keyring: name.to_string(),
            fallback: name.to_string(),
        }
    }
}
const FALLBACK_KEY_FILE: &str = ".key";
const FALLBACK_PW_FILE: &str = ".pw";

/// Store a password for a host.
///
/// First tries the OS keyring. If the keyring is unavailable, falls back to an
/// encrypted file in the application data directory so passwords still persist
/// across sessions on headless / minimal Linux desktops without a secret service.
pub fn store_password(host_id: i64, password: &str) -> Result<(), String> {
    let base_dir = data_dir()?;
    store_password_internal(&Account::host(host_id), password, &base_dir)
}

/// Store a secret under an explicit account name, using the same keyring and
/// encrypted-file fallback as host passwords.
pub fn store_secret(account: &str, secret: &str) -> Result<(), String> {
    let base_dir = data_dir()?;
    store_password_internal(&Account::named(account), secret, &base_dir)
}

/// Retrieve a secret stored by [`store_secret`].
pub fn get_secret(account: &str) -> Result<String, String> {
    let base_dir = data_dir()?;
    get_password_internal(&Account::named(account), &base_dir)
}

/// Remove a secret stored by [`store_secret`] from both stores.
pub fn delete_secret(account: &str) -> Result<(), String> {
    let base_dir = data_dir()?;
    delete_password_internal(&Account::named(account), &base_dir)
}

fn store_password_internal(
    account: &Account,
    password: &str,
    base_dir: &Path,
) -> Result<(), String> {
    let keyring_result =
        Entry::new(SERVICE_NAME, &account.keyring).and_then(|entry| entry.set_password(password));

    match keyring_result {
        Ok(()) => {
            // Keyring worked — clear any stale fallback entry for this account.
            let _ = fallback_delete_password(account, base_dir);
            Ok(())
        }
        Err(keyring_err) => {
            // Fall back to encrypted file storage.
            fallback_store_password(account, password, base_dir).map_err(|fallback_err| {
                format!(
                    "keyring store failed: {}; fallback store failed: {}",
                    keyring_err, fallback_err
                )
            })
        }
    }
}

/// Retrieve a password for a host.
///
/// First tries the OS keyring, then falls back to the encrypted file store.
pub fn get_password(host_id: i64) -> Result<String, String> {
    let base_dir = data_dir()?;
    get_password_internal(&Account::host(host_id), &base_dir)
}

fn get_password_internal(account: &Account, base_dir: &Path) -> Result<String, String> {
    let keyring_result =
        Entry::new(SERVICE_NAME, &account.keyring).and_then(|entry| entry.get_password());

    match keyring_result {
        Ok(password) => Ok(password),
        Err(keyring_err) => fallback_get_password(account, base_dir).map_err(|fallback_err| {
            format!(
                "keyring retrieve failed: {}; fallback retrieve failed: {}",
                keyring_err, fallback_err
            )
        }),
    }
}

/// Delete a stored password for a host.
///
/// Removes from both the OS keyring and the fallback file store; errors are
/// ignored because the goal is simply to ensure no copy remains.
pub fn delete_password(host_id: i64) -> Result<(), String> {
    let base_dir = data_dir()?;
    delete_password_internal(&Account::host(host_id), &base_dir)
}

fn delete_password_internal(account: &Account, base_dir: &Path) -> Result<(), String> {
    if let Ok(entry) = Entry::new(SERVICE_NAME, &account.keyring) {
        let _ = entry.delete_credential();
    }
    let _ = fallback_delete_password(account, base_dir);
    Ok(())
}

fn data_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|d| d.join("termdrop"))
        .ok_or_else(|| "could not determine application data directory".to_string())
}

fn load_or_create_fallback_key(base_dir: &Path) -> Result<[u8; 32], String> {
    std::fs::create_dir_all(base_dir).map_err(|e| format!("create data dir: {}", e))?;
    let key_path = base_dir.join(FALLBACK_KEY_FILE);

    if key_path.exists() {
        let b64 = std::fs::read_to_string(&key_path)
            .map_err(|e| format!("read fallback key file: {}", e))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("decode fallback key: {}", e))?;
        if bytes.len() != 32 {
            return Err("invalid fallback key length".to_string());
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(key);
    }

    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let b64 = base64::engine::general_purpose::STANDARD.encode(key);
    std::fs::write(&key_path, b64).map_err(|e| format!("write fallback key file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&key_path)
            .map_err(|e| format!("read fallback key permissions: {}", e))?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&key_path, perms)
            .map_err(|e| format!("set fallback key permissions: {}", e))?;
    }

    Ok(key)
}

fn fallback_store_password(
    account: &Account,
    password: &str,
    base_dir: &Path,
) -> Result<(), String> {
    let key = load_or_create_fallback_key(base_dir)?;
    let ciphertext = encrypt(password, &key)?;
    let path = base_dir.join(FALLBACK_PW_FILE);

    let mut map: HashMap<String, String> = if path.exists() {
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("read fallback password file: {}", e))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("parse fallback password file: {}", e))?
    } else {
        HashMap::new()
    };

    map.insert(account.fallback.clone(), ciphertext);
    let content = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("serialize fallback password file: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("write fallback password file: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)
            .map_err(|e| format!("read fallback password permissions: {}", e))?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .map_err(|e| format!("set fallback password permissions: {}", e))?;
    }

    Ok(())
}

fn fallback_get_password(account: &Account, base_dir: &Path) -> Result<String, String> {
    let key = load_or_create_fallback_key(base_dir)?;
    let path = base_dir.join(FALLBACK_PW_FILE);
    if !path.exists() {
        return Err("no fallback password file".to_string());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read fallback password file: {}", e))?;
    let map: HashMap<String, String> = serde_json::from_str(&content)
        .map_err(|e| format!("parse fallback password file: {}", e))?;
    let ciphertext = map
        .get(&account.fallback)
        .ok_or_else(|| "no fallback password for host".to_string())?;
    decrypt(ciphertext, &key)
}

fn fallback_delete_password(account: &Account, base_dir: &Path) -> Result<(), String> {
    let path = base_dir.join(FALLBACK_PW_FILE);
    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read fallback password file: {}", e))?;
    let mut map: HashMap<String, String> = serde_json::from_str(&content)
        .map_err(|e| format!("parse fallback password file: {}", e))?;
    map.remove(&account.fallback);

    let content = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("serialize fallback password file: {}", e))?;
    std::fs::write(&path, content).map_err(|e| format!("write fallback password file: {}", e))?;
    Ok(())
}

fn encrypt(plaintext: &str, key: &[u8; 32]) -> Result<String, String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("initialize cipher: {:?}", e))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("encrypt: {:?}", e))?;

    let mut combined = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(base64::engine::general_purpose::STANDARD.encode(&combined))
}

fn decrypt(b64: &str, key: &[u8; 32]) -> Result<String, String> {
    let combined = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("decode ciphertext: {}", e))?;
    if combined.len() < 12 {
        return Err("ciphertext too short".to_string());
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("initialize cipher: {:?}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt: {:?}", e))?;
    String::from_utf8(plaintext).map_err(|e| format!("utf8: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_accounts_keep_their_historical_key_shapes() {
        // These two strings are on disk in existing installs. The keyring
        // account and the fallback-file key differ, and changing either would
        // orphan every SSH password already stored.
        let a = Account::host(42);
        assert_eq!(a.keyring, "host-42");
        assert_eq!(a.fallback, "42");
    }

    #[test]
    fn named_accounts_cannot_collide_with_host_ids() {
        let named = Account::named("mongo-remote-42");
        assert_eq!(named.fallback, "mongo-remote-42");
        assert_ne!(named.fallback, Account::host(42).fallback);
    }

    #[test]
    fn named_secrets_round_trip_independently_of_host_passwords() {
        let temp = std::env::temp_dir().join(format!(
            "termdrop-crypto-named-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp).unwrap();

        let host = Account::host(7);
        let mongo = Account::named("mongo-remote-7");

        store_password_internal(&host, "ssh-secret", &temp).unwrap();
        store_password_internal(&mongo, "mongo-secret", &temp).unwrap();

        // Same numeric id, different secrets, neither clobbering the other.
        assert_eq!(get_password_internal(&host, &temp).unwrap(), "ssh-secret");
        assert_eq!(
            get_password_internal(&mongo, &temp).unwrap(),
            "mongo-secret"
        );

        // Deleting one leaves the other intact.
        delete_password_internal(&mongo, &temp).unwrap();
        assert_eq!(get_password_internal(&host, &temp).unwrap(), "ssh-secret");

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_fallback_password_round_trip() {
        let temp = std::env::temp_dir().join(format!(
            "termdrop-crypto-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp).unwrap();

        let host_id = 42i64;
        let password = "my-s3cret-p@ssw0rd!";

        // In this environment the OS keyring (D-Bus/Secret Service) is expected
        // to be unavailable, so the internal functions should transparently use
        // the encrypted fallback store.
        let account = Account::host(host_id);
        store_password_internal(&account, password, &temp).expect("store should succeed");
        let retrieved = get_password_internal(&account, &temp).expect("retrieve should succeed");
        assert_eq!(retrieved, password);

        delete_password_internal(&account, &temp).expect("delete should succeed");
        assert!(get_password_internal(&account, &temp).is_err());

        // Clean up.
        let _ = std::fs::remove_dir_all(&temp);
    }
}
