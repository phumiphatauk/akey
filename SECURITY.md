# Security Policy

AKey handles 2FA secrets, so security reports are taken seriously.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through
[GitHub's private vulnerability reporting](../../security/advisories/new) instead.
Include steps to reproduce and the macOS and AKey versions you tested on.

## Scope

In scope: anything that could expose secrets or codes, such as secrets written outside the
Keychain, leaked to logs, the clipboard, or shell history, or readable by other apps.

Out of scope: attacks that already require an unlocked session as your user with Keychain
access granted to AKey.
