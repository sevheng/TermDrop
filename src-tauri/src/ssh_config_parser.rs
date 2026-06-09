use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SshConfigHost {
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    pub key_path: Option<String>,
}

/// Parse ~/.ssh/config and return a list of importable hosts.
/// Skips wildcard-only blocks (e.g., `Host *`).
pub fn parse_ssh_config() -> Result<Vec<SshConfigHost>, String> {
    let config_path = dirs::home_dir()
        .map(|h| h.join(".ssh/config"))
        .unwrap_or_else(|| PathBuf::from(".ssh/config"));

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {:?}: {}", config_path, e))?;

    let mut hosts = Vec::new();
    let mut current_patterns: Vec<String> = Vec::new();
    let mut current_fields: Vec<(String, String)> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.to_lowercase().starts_with("host ") {
            // Flush previous block
            flush_block(&mut hosts, &current_patterns, &current_fields);
            current_patterns.clear();
            current_fields.clear();

            let patterns = line[5..].trim();
            for pattern in patterns.split_whitespace() {
                current_patterns.push(pattern.to_string());
            }
            continue;
        }

        if !current_patterns.is_empty() {
            let mut parts = line.splitn(2, char::is_whitespace);
            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                current_fields.push((key.to_lowercase(), value.trim().to_string()));
            }
        }
    }

    // Flush last block
    flush_block(&mut hosts, &current_patterns, &current_fields);

    Ok(hosts)
}

fn flush_block(
    hosts: &mut Vec<SshConfigHost>,
    patterns: &[String],
    fields: &[(String, String)],
) {
    if patterns.is_empty() {
        return;
    }

    // Skip pure wildcard blocks
    let concrete_patterns: Vec<&String> = patterns
        .iter()
        .filter(|p| !is_wildcard(p))
        .collect();

    if concrete_patterns.is_empty() {
        return;
    }

    // Extract fields (case-insensitive keys)
    let mut hostname = None;
    let mut user = None;
    let mut port = 22i64;
    let mut identity_file = None;

    for (key, value) in fields {
        match key.as_str() {
            "hostname" => hostname = Some(value.clone()),
            "user" => user = Some(value.clone()),
            "port" => {
                if let Ok(p) = value.parse::<i64>() {
                    port = p;
                }
            }
            "identityfile" => identity_file = Some(expand_tilde(value)),
            _ => {}
        }
    }

    for pattern in concrete_patterns {
        let name = pattern.clone();
        // If HostName is not set, use the pattern itself as the address
        let host = hostname.clone().unwrap_or_else(|| pattern.clone());
        let username = user.clone().unwrap_or_else(|| whoami::username().unwrap_or_else(|_| "user".to_string()));
        let auth_type = if identity_file.is_some() { "key" } else { "password" };

        hosts.push(SshConfigHost {
            name,
            host,
            port,
            username,
            auth_type: auth_type.to_string(),
            key_path: identity_file.clone(),
        });
    }
}

fn is_wildcard(pattern: &str) -> bool {
    pattern == "*" || pattern.contains('*') || pattern.contains('?')
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&path[2..]).to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    }
}
