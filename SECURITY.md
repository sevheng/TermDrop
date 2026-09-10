# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.3.x   | :white_check_mark: |
| 0.2.x   | :x:                |
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

- Passwords are stored in the OS keyring (Keychain on macOS, Credential Manager
  on Windows, Secret Service on Linux), or in an AES-GCM encrypted file under the
  application data directory when no keyring is available — never in the local
  SQLite database and never in any cloud service.
- All SSH connections use the `ssh2` crate with standard OpenSSL/libssh2 encryption.
- MongoDB and Redis passwords follow the same rule: the connection string kept in
  the local SQLite database holds only the username, and host exports are
  stripped of passwords — importing an older export discards them rather than
  writing plaintext back.
- Connection strings are redacted in log output, and are never rendered into
  tooltips or list rows; only host and port are shown. MongoDB credentials are
  passed to `mongodump`/`mongorestore` through a `--config` file created `0600`
  and deleted afterwards, rather than on the command line where any local user
  could read them from `ps`.
- Redis and MongoDB browsing are read-only: there is no command console and no
  edit path, so nothing typed in either panel can modify a database.
- Redis backup files carry no credentials, and a truncated or damaged one is
  refused rather than restored in part — every length is validated against a cap
  before anything is allocated, and a record count and CRC32 are checked before
  the first key is written.
- An SSH tunnel to a Redis server binds `127.0.0.1` only and closes with the tab.
  `rediss://` through a tunnel is refused outright rather than silently
  downgraded: the driver would see `127.0.0.1` and certificate verification could
  never succeed.
- Security-audit remediation commands are module literals selected by OS family
  and never interpolate output read from the remote host, and nothing the app
  types into a terminal is submitted — the user presses Enter.
- No telemetry, analytics, or cloud sync.
- All data remains local to the user's machine.

## Disclosure Policy

We follow a coordinated disclosure process:
1. Reporter submits vulnerability privately.
2. We acknowledge receipt within 48 hours.
3. We investigate and develop a fix.
4. We release a patched version and publish a security advisory.
5. We publicly disclose the vulnerability with full details after users have had reasonable time to update.
