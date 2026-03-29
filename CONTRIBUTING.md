# Contributing to mcp-vault-wrap

## Development Process

- **Branching:** `main` is always buildable. Use feature branches for all changes, merged via pull request.
- **Commit messages:** Use conventional commit format: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- **Pull requests:** All changes go through PRs, even from maintainers. PRs should reference relevant issues.

## Registry Changes

Registry changes (adding or modifying MCP server definitions) are security-sensitive. Every registry change must include:

- Per-variable classification rationale (`secret` vs `config`)
- Migration impact note for existing users

See [docs/mvp-architecture-contract.md](docs/mvp-architecture-contract.md) §7 for the full governance model.

## Reporting Security Issues

See [SECURITY.md](SECURITY.md) for how to report vulnerabilities. Do not open public issues for security bugs.

## License

By contributing, you agree that your contributions will be dual-licensed under MIT and Apache 2.0, consistent with the project's licensing.
