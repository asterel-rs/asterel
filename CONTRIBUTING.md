# Contributing

Asterel is in active pre-release development. Contributions are welcome when they
fit the companion-first scope and preserve the runtime's safety boundaries.

## Before opening an issue or pull request

- Read `README.md` for the current product scope and command surface.
- Read `SECURITY.md` before reporting anything that could expose secrets,
  private memory, gateway/admin access, tool execution, or external ingress.
- Do not post provider keys, OAuth tokens, channel tokens, private Discord logs,
  memory payloads, workspace paths that identify another person, or local secret
  material in public issues or pull requests.
- Check the public docs site and executable source before making design or
  architecture claims; source and CI behavior win over stale prose.

## What fits this repository

Good first contributions are usually narrow:

- reproducible bug reports with commands, OS, version, and redacted logs;
- documentation corrections that align README / public docs / current source;
- tests for existing companion-turn, memory, gateway, channel, or security
  behavior;
- small fixes that keep transports thin and preserve the shared companion turn
  contract.

Avoid broad rewrites, compatibility shims, planning-first product framing, or new
large design memos unless maintainers have agreed on the scope first.

## Pull request expectations

Use the pull request template and explain why the change exists, not only what
files changed. For code changes, run the narrowest relevant tests first and then
the repo baseline when practical:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check-all
cargo test
```

Desktop changes use `pnpm` in `desktop/`:

```bash
pnpm exec oxfmt
pnpm exec oxlint --react-plugin src
pnpm build
```

Generated artifacts must be committed when the source that generates them
changes. Do not hand-edit `desktop/src/routeTree.gen.ts`.

## Security reports

Do not open public issues for vulnerabilities. Use GitHub private vulnerability
reporting / repository security advisories when enabled:

<https://github.com/asterel-rs/asterel/security/advisories/new>

If that form is not available, open a public issue asking for a private security
contact without including vulnerability details.

## License note

Asterel is dual-licensed under MIT or Apache-2.0, at your option. Unless you
explicitly state otherwise, any contribution intentionally submitted for
inclusion in Asterel is licensed the same way.
