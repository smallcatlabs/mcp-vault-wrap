# Release Process

## Prerequisites

- GPG key configured for git tag signing (`git config --global user.signingkey <key-id>`)
- Push access to the repository
- macOS machine for smoke tests (Keychain access required)

## Steps

### 1. Run smoke tests on macOS

```bash
cargo build --release
./tests/smoke/smoke-run.sh
./tests/smoke/smoke-migrate.sh
```

Both scripts must report all checks passed before proceeding.

### 2. Verify CI is green

Ensure the latest commit on `main` passes all CI checks (build, test, fmt, clippy, audit).

### 3. Create a signed tag

```bash
git tag -s v<VERSION> -m "v<VERSION>"
```

Verify the signature:

```bash
git tag -v v<VERSION>
```

### 4. Push the tag

```bash
git push origin v<VERSION>
```

This triggers the release workflow (`.github/workflows/release.yml`), which:

- Builds macOS binaries for aarch64 and x86_64
- Runs `cargo audit`
- Generates a CycloneDX SBOM
- Computes SHA-256 checksums for each binary
- Creates a GitHub Release with all artifacts attached

### 5. Verify the release

After the workflow completes:

- [ ] GitHub Release exists with correct tag
- [ ] Both macOS binaries are attached (aarch64 + x86_64)
- [ ] `checksums-sha256.txt` is attached and contains both binaries
- [ ] SBOM (`mcp-vault-wrap-sbom.cdx.json`) is attached
- [ ] Release notes are accurate

### 6. Verify binary checksums

Download an artifact and verify:

```bash
shasum -a 256 mcp-vault-wrap-aarch64-apple-darwin.tar.gz
# Compare against checksums-sha256.txt
```

## Version Numbering

Follow [Semantic Versioning](https://semver.org/). Update `version` in `Cargo.toml` before tagging.

## Security Claims

Every release must include current "Protects Against" and "Does Not Protect Against" claims (see `SECURITY.md`). If claims change from the previous release, add a **Security Model Changes** section to the release notes.
