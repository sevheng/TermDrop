use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

    let default_user = whoami::username().unwrap_or_else(|_| "user".to_string());
    Ok(parse_ssh_config_str(
        &content,
        &default_user,
        dirs::home_dir().as_deref(),
    ))
}

/// Parse ssh_config text. `default_user` fills in hosts without a `User`
/// line; `home_dir` expands a leading `~/` in `IdentityFile`.
fn parse_ssh_config_str(
    content: &str,
    default_user: &str,
    home_dir: Option<&Path>,
) -> Vec<SshConfigHost> {
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
            flush_block(
                &mut hosts,
                &current_patterns,
                &current_fields,
                default_user,
                home_dir,
            );
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
    flush_block(
        &mut hosts,
        &current_patterns,
        &current_fields,
        default_user,
        home_dir,
    );

    hosts
}

fn flush_block(
    hosts: &mut Vec<SshConfigHost>,
    patterns: &[String],
    fields: &[(String, String)],
    default_user: &str,
    home_dir: Option<&Path>,
) {
    if patterns.is_empty() {
        return;
    }

    // Skip pure wildcard blocks
    let concrete_patterns: Vec<&String> = patterns.iter().filter(|p| !is_wildcard(p)).collect();

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
            "identityfile" => identity_file = Some(expand_tilde(value, home_dir)),
            _ => {}
        }
    }

    for pattern in concrete_patterns {
        let name = pattern.clone();
        // If HostName is not set, use the pattern itself as the address
        let host = hostname.clone().unwrap_or_else(|| pattern.clone());
        let username = user.clone().unwrap_or_else(|| default_user.to_string());
        let auth_type = if identity_file.is_some() {
            "key"
        } else {
            "password"
        };

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

fn expand_tilde(path: &str, home_dir: Option<&Path>) -> String {
    if path.starts_with("~/") {
        home_dir
            .map(|h| h.join(&path[2..]).to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = include_str!("fixtures/ssh_config.txt");

    fn parse() -> Vec<SshConfigHost> {
        parse_ssh_config_str(CONFIG, "fallback", Some(Path::new("/home/tester")))
    }

    #[test]
    fn skips_wildcard_only_blocks() {
        let hosts = parse();
        let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["prod", "prod-alias", "staging", "mixed", "concrete"]
        );
    }

    #[test]
    fn each_concrete_pattern_becomes_a_host_sharing_the_block_fields() {
        let hosts = parse();
        for h in &hosts[..2] {
            assert_eq!(h.host, "10.0.0.5");
            assert_eq!(h.username, "deploy");
            assert_eq!(h.port, 2222);
            assert_eq!(h.auth_type, "key");
        }
    }

    #[test]
    fn expands_tilde_in_identity_file_and_keeps_absolute_paths() {
        let hosts = parse();
        assert_eq!(
            hosts[0].key_path.as_deref(),
            Some("/home/tester/.ssh/id_prod")
        );
        assert_eq!(hosts[3].key_path.as_deref(), Some("/abs/key"));
        assert_eq!(
            expand_tilde("~/.ssh/x", None),
            "~/.ssh/x",
            "no home dir leaves the path untouched"
        );
    }

    #[test]
    fn lowercases_keys_and_applies_defaults() {
        let hosts = parse();
        let staging = &hosts[2];
        assert_eq!(staging.host, "staging.example.com");
        assert_eq!(staging.username, "fallback");
        assert_eq!(staging.port, 22);
        assert_eq!(staging.auth_type, "password");
        assert_eq!(staging.key_path, None);
    }

    #[test]
    fn pattern_is_used_as_host_when_hostname_missing() {
        let hosts = parse();
        assert_eq!(hosts[4].name, "concrete");
        assert_eq!(hosts[4].host, "concrete");
        assert_eq!(hosts[4].username, "ops");
    }

    #[test]
    fn invalid_port_keeps_default() {
        let hosts = parse_ssh_config_str("Host a\n  Port notanumber\n", "u", None);
        assert_eq!(hosts[0].port, 22);
    }
}
