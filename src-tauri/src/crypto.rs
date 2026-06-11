use keyring::Entry;

const SERVICE_NAME: &str = "termdrop";

pub fn store_password(host_id: i64, password: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &format!("host-{}", host_id))
        .map_err(|e| format!("keyring entry creation failed: {}", e))?;
    entry
        .set_password(password)
        .map_err(|e| format!("keyring store failed: {}", e))?;
    Ok(())
}

pub fn get_password(host_id: i64) -> Result<String, String> {
    let entry = Entry::new(SERVICE_NAME, &format!("host-{}", host_id))
        .map_err(|e| format!("keyring entry creation failed: {}", e))?;
    entry
        .get_password()
        .map_err(|e| format!("keyring retrieve failed: {}", e))
}

pub fn delete_password(host_id: i64) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &format!("host-{}", host_id))
        .map_err(|e| format!("keyring entry creation failed: {}", e))?;
    entry
        .delete_credential()
        .map_err(|e| format!("keyring delete failed: {}", e))?;
    Ok(())
}
