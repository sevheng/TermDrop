# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.2.x   | :white_check_mark: |
| 0.1.x   | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in TermDrop, please report it responsibly.

**Please do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please send an email to **sevhengluy@gmail.com** with:
- A description of the vulnerability
- Steps to reproduce (if applicable)
- Potential impact
- Any suggested fixes

We aim to respond within 48 hours and will work with you to verify, address, and disclose the issue appropriately.

## Security Design

- Passwords are stored exclusively in the OS keyring (Keychain on macOS, Credential Manager on Windows, Secret Service on Linux) — never in the local SQLite database or any cloud service.
- All SSH connections use the `ssh2` crate with standard OpenSSL/libssh2 encryption.
- MongoDB connection URIs (including credentials) are stored in the local SQLite database. Use connection strings with read-only users where possible.
- No telemetry, analytics, or cloud sync.
- All data remains local to the user's machine.

## Disclosure Policy

We follow a coordinated disclosure process:
1. Reporter submits vulnerability privately.
2. We acknowledge receipt within 48 hours.
3. We investigate and develop a fix.
4. We release a patched version and publish a security advisory.
5. We publicly disclose the vulnerability with full details after users have had reasonable time to update.
