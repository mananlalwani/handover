# Security policy

## Reporting a vulnerability

Do not report security issues in a public GitHub issue. Use GitHub's private
vulnerability reporting for this repository. Include a short description, the
affected revision, reproduction steps that do not contain secrets, and the
impact. If private reporting is unavailable, contact the maintainers privately
before sharing details.

Never include pairing codes, private keys, tokens, cookies, credential bundles,
message contents, notification text, file contents, phone numbers, or sensitive
logs in a public issue, pull request, or discussion.

## Scope

Reports about native TLS and pairing, Android identity and storage, daemon IPC,
backend isolation, secret handling, or sensitive logging are in scope. Reports
about the optional Google Messages integration should include only sanitized
details. Google authentication and relay credentials belong to the external
`handover-gmessages` adapter and must not be pasted into this repository.

The MIT daemon and the AGPL adapter run in separate processes and communicate
through the [helper contract](docs/gmessages-sidecar.md). This describes the
engineering boundary; licensing obligations depend on how the software is
distributed and deployed.
