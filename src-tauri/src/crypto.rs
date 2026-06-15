use base64::Engine;
use keyring::Entry;
use rand::RngCore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};

const SERVICE_NAME: &str = "termdrop";
const FALLBACK_KEY_FILE: &str = ".key";
const FALLBACK_PW_FILE: &str = ".pw";

/// Store a password for a host.
///
/// First tries the OS keyring. If the keyring is unavailable, falls back to an
/// encrypted file in the application data directory so passwords still persist
/// across sessions on headless / minimal Linux desktops without a secret service.
pub fn store_password(host_id: i64, password: &str) -> Result<(), String> {
    let base_dir = data_dir()?;
    store_password_internal(host_id, password, &base_dir)
}

fn store_password_internal(host_id: i64, password: &str, base_dir: &Path) -> Result<(), String> {
    let keyring_result = Entry::new(SERVICE_NAME, &format!("host-{}", host_id))
        .and_then(|entry| entry.set_password(password));

    match keyring_result {
        Ok(()) => {
            // Keyring worked — clear any stale fallback entry for this host.
            let _ = fallback_delete_password(host_id, base_dir);
            Ok(())
        }
        Err(keyring_err) => {
            // Fall back to encrypted file storage.
            fallback_store_password(host_id, password, base_dir).map_err(|fallback_err| {
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
    get_password_internal(host_id, &base_dir)
}

fn get_password_internal(host_id: i64, base_dir: &Path) -> Result<String, String> {
    let keyring_result = Entry::new(SERVICE_NAME, &format!("host-{}", host_id))
        .and_then(|entry| entry.get_password());

    match keyring_result {
        Ok(password) => Ok(password),
        Err(keyring_err) => fallback_get_password(host_id, base_dir).map_err(|fallback_err| {
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
    delete_password_internal(host_id, &base_dir)
}

fn delete_password_internal(host_id: i64, base_dir: &Path) -> Result<(), String> {
    if let Ok(entry) = Entry::new(SERVICE_NAME, &format!("host-{}", host_id)) {
        let _ = entry.delete_credential();
    }
    let _ = fallback_delete_password(host_id, base_dir);
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
    let b64 = base64::engine::general_purpose::STANDARD.encode(&key);
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

fn fallback_store_password(host_id: i64, password: &str, base_dir: &Path) -> Result<(), String> {
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

    map.insert(host_id.to_string(), ciphertext);
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

fn fallback_get_password(host_id: i64, base_dir: &Path) -> Result<String, String> {
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
        .get(&host_id.to_string())
        .ok_or_else(|| "no fallback password for host".to_string())?;
    decrypt(ciphertext, &key)
}

fn fallback_delete_password(host_id: i64, base_dir: &Path) -> Result<(), String> {
    let path = base_dir.join(FALLBACK_PW_FILE);
    if !path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read fallback password file: {}", e))?;
    let mut map: HashMap<String, String> = serde_json::from_str(&content)
        .map_err(|e| format!("parse fallback password file: {}", e))?;
    map.remove(&host_id.to_string());

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
        store_password_internal(host_id, password, &temp).expect("store should succeed");
        let retrieved = get_password_internal(host_id, &temp).expect("retrieve should succeed");
        assert_eq!(retrieved, password);

        delete_password_internal(host_id, &temp).expect("delete should succeed");
        assert!(get_password_internal(host_id, &temp).is_err());

        // Clean up.
        let _ = std::fs::remove_dir_all(&temp);
    }
}
