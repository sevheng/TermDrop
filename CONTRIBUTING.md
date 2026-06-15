# Contributing to TermDrop

Thank you for your interest in contributing to **TermDrop**! This guide covers how to report issues, propose features, and submit pull requests.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How to Contribute](#how-to-contribute)
  - [Reporting Bugs](#reporting-bugs)
  - [Suggesting Features](#suggesting-features)
  - [Pull Requests](#pull-requests)
- [Development Setup](#development-setup)
  - [Prerequisites](#prerequisites)
  - [Fork and Clone](#fork-and-clone)
  - [Install Dependencies](#install-dependencies)
  - [Run Locally](#run-locally)
  - [Build for Production](#build-for-production)
- [Project Structure](#project-structure)
- [Coding Guidelines](#coding-guidelines)
  - [Commits](#commits)
  - [Branches](#branches)
  - [Pull Request Best Practices](#pull-request-best-practices)
- [Before Submitting](#before-submitting)
- [Security Issues](#security-issues)
- [License](#license)

---

## Code of Conduct

Be respectful, constructive, and inclusive. We welcome contributors of all experience levels. If you are unsure about anything, open an issue and ask — we are happy to help.

---

## How to Contribute

### Reporting Bugs

If you find a bug, please open a [GitHub Issue](https://github.com/sevheng/TermDrop/issues) and include:

- A clear, descriptive title
- Steps to reproduce the problem
- Expected behavior vs. actual behavior
- Your operating system and TermDrop version
- Screenshots or logs, if applicable

### Suggesting Features

Feature requests are welcome! Before opening a new issue, check whether someone has already suggested something similar. When proposing a feature, describe:

- The problem you are trying to solve
- Your proposed solution
- Any alternatives you have considered

### Pull Requests

We use the standard fork-and-pull workflow:

1. **Fork** the repository on GitHub.
2. **Clone** your fork locally.
3. Create a new branch from `main`.
4. Make your changes.
5. Run a local build to verify everything works.
6. Push your branch to your fork.
7. Open a **Pull Request** against the `main` branch of `sevheng/TermDrop`.

---

## Development Setup

### Prerequisites

- [Node.js](https://nodejs.org/) 18 or later
- [Rust](https://rustup.rs/) stable toolchain
- A GitHub account

### Fork and Clone

```bash
# Fork the repo on GitHub first, then clone your fork
git clone git@github.com:YOUR_USERNAME/TermDrop.git
cd TermDrop/ssh-client
```

Add the upstream repository as a remote to keep your fork in sync:

```bash
git remote add upstream git@github.com:sevheng/TermDrop.git
git fetch upstream
```

### Install Dependencies

```bash
npm install
```

### Run Locally

```bash
npm run tauri dev
```

This starts the Vite dev server and the Tauri application with hot reload.

### Build for Production

```bash
npm run tauri build
```

Output locations:

- Windows: `src-tauri/target/release/bundle/msi/*.msi`
- macOS: `src-tauri/target/release/bundle/dmg/*.dmg`
- Linux: `src-tauri/target/release/bundle/appimage/*.AppImage` or `*.deb`

---

## Project Structure

```
ssh-client/
├── src/              # Vue 3 frontend (components, stores, composables)
├── src-tauri/        # Rust backend and Tauri commands
│   ├── src/          # Rust source code
│   └── Cargo.toml    # Rust dependencies
├── public/           # Static assets
├── index.html
├── package.json
└── README.md
```

- Frontend code lives in `src/` and uses Vue 3 + TypeScript.
- Backend code and native commands live in `src-tauri/src/` and are written in Rust.

---

## Coding Guidelines

### Commits

We recommend following [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add drag-and-drop upload to SFTP browser
fix: prevent terminal tab crash on disconnect
docs: update keyboard shortcut table
refactor: simplify host store state management
```

### Branches

Use descriptive branch names:

- `feature/terminal-search-history`
- `fix/sftp-upload-progress`
- `docs/contributing-guide`

### Pull Request Best Practices

- Keep pull requests focused and small.
- One logical change per PR.
- Explain the "why" in your PR description, not just the "what".
- Reference related issues with `Closes #123` or `Fixes #123` when applicable.

---

## Before Submitting

Before opening a pull request, please:

- [ ] Build the project successfully with `npm run tauri build`
- [ ] Verify the dev app runs with `npm run tauri dev`
- [ ] Update relevant documentation if your change affects behavior
- [ ] Keep the code style consistent with the existing codebase

---

## Security Issues

Please do **not** open public issues for security vulnerabilities. Instead, follow the instructions in [`SECURITY.md`](SECURITY.md).

---

## License

By contributing to TermDrop, you agree that your contributions will be licensed under the [MIT License](LICENSE).
