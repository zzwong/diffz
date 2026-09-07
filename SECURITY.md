# Security Policy

## Supported versions

Security fixes land on the `main` branch and on the latest release.

| Version | Supported |
| ------- | --------- |
| latest release | yes |
| main | yes |
| older releases | no |

## Reporting a vulnerability

Never report a security problem in a public issue.

This repository has GitHub's private vulnerability reporting; find it in the
Security tab under "Report a vulnerability". Expect a reply within 7 days.

### Scope

diffz runs on your desktop. Your data goes nowhere on its own; it only travels
through `gh` (GitHub) or `glab` (GitLab), the CLI tools you installed yourself.
Publishing reviews or comments counts as a write operation and happens only
after you pass the matching opt-in flags.

If diffz leaks data to third parties, or writes anything without opt-in,
report it. Reports like that are welcome and in scope.
