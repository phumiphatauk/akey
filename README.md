# AKey

[![CI](https://github.com/phumiphatauk/akey/actions/workflows/ci.yml/badge.svg)](https://github.com/phumiphatauk/akey/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![macOS 14+](https://img.shields.io/badge/macOS-14%2B-lightgrey)

A minimal 2FA (TOTP) code generator for the macOS menu bar. It is written in Rust on native AppKit (via [`objc2`](https://github.com/madsmtm/objc2)), with no WebView, no async runtime, and no network access. The release binary is about 430 KB.

## Features

- **One click to copy:** click 🔑 to see every account's code with a countdown ring, then click a row to copy it. The code is computed at the moment you click, so it is never stale.
- **Keychain only:** secrets live in your login keychain and nowhere else.
- **Clipboard hygiene:** 30 seconds after copying, the clipboard is cleared if it still holds the code AKey put there. Anything you copied since is left alone.
- **Manage accounts in the app or a CLI:** add, rename, re-key, reorder (by dragging), and delete accounts.
- **Standard `otpauth://` support:** SHA1/SHA256/SHA512, 6–8 digits, and custom periods.
- **Zero cost when idle:** while the popover is closed, AKey runs no timers (0% CPU, 0 idle wakeups). Views are created on open and discarded on close, and the rings are Core Animation animations that the system renders.

## Requirements

- macOS 14 Sonoma or later
- Rust (stable) and the Xcode Command Line Tools to build

## Build and install

```sh
git clone https://github.com/phumiphatauk/akey.git
cd akey
./scripts/bundle.sh       # builds target/release/bundle.noindex/AKey.app (ad-hoc signed)

cp -R target/release/bundle.noindex/AKey.app /Applications/
open /Applications/AKey.app
```

To use the CLI, link the binary inside the app bundle. Using the same binary lets the Keychain treat the CLI and the app as one program:

```sh
sudo mkdir -p /usr/local/bin
sudo ln -sf /Applications/AKey.app/Contents/MacOS/akey /usr/local/bin/akey
```

To start AKey at login, go to System Settings → General → Login Items & Extensions → **Open at Login**, click **+**, and choose `/Applications/AKey.app`.

## Usage

### Menu bar

```
        [+]   [✎]   [⏻]
 07 66 84              ◔      ← click a row to copy; shows "✓ Copied!" and the popover closes
 GitHub
```

| Control | Action |
| --- | --- |
| **+** | Add an account. Paste the setup key (Base32) or the `otpauth://…` link from a site's 2FA setup page. If you paste a link and leave the name empty, the issuer name is used. |
| **✎** | Edit mode. Each row gets a button to rename the account or paste a new secret, and a delete button that asks for confirmation inline. |
| **⏻** | Quit. |
| Drag a row | Reorder. The new order is saved when you release. A click without moving the mouse counts as a copy. |

- While the add or edit form is open, the popover stays open, so you can switch to your browser to copy a key. Return saves and Esc cancels.
- Text fields support ⌘V, ⌘C, ⌘X, ⌘A, and ⌘Z.
- New accounts are added at the bottom of the list.
- The rings use the system accent color, and the popover follows the menu bar's light or dark appearance.

### Command line

```sh
akey add github          # prompts for the secret with echo off
                         # accepts Base32 (JBSW Y3DP ...) or otpauth://totp/...?secret=...
pbpaste | akey add aws   # or read it from stdin
akey list                # account names only, never secrets
akey rename github "GitHub (work)"
akey move github 1       # move to the top (same order as the popover)
akey remove github
```

- `add` prints the current code. Check that it matches the website before you confirm 2FA there.
- Account names can't start with `.`, because AKey stores the ordering in a Keychain item named `.order`.
- The CLI works while the app is running. The popover re-reads the Keychain every time it opens.

## Security

- Secrets are stored only in the macOS login keychain. The account order is stored there as well, and it holds names only.
- The CLI never accepts a secret as an argument, so secrets don't end up in shell history or `ps` output. Secret buffers are zeroed after parsing.
- AKey makes no network connections.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

### Keychain access prompts

Ad-hoc signatures are tied to the binary's hash, so **after every rebuild** macOS asks *"AKey wants to use your confidential information stored in …"* the first time you open the popover. Enter your login password and click **Always Allow**. macOS asks once per account.

To stop the prompts after rebuilds, sign with a stable identity instead of `-` in [`scripts/bundle.sh`](scripts/bundle.sh). A self-signed code signing certificate created in Keychain Access works.

## Development

```sh
cargo test                # RFC 4226/6238 test vectors, otpauth:// parsing, ordering
cargo clippy --all-targets
```

These environment variables let you work on the UI without touching the real Keychain:

```sh
open -n --env AKEY_DEMO=1 --env AKEY_OPEN_ON_LAUNCH=list target/release/bundle.noindex/AKey.app
```

| Variable | Effect |
| --- | --- |
| `AKEY_DEMO=1` | Use in-memory sample accounts instead of the Keychain. Writes are ignored. |
| `AKEY_OPEN_ON_LAUNCH=list\|edit\|confirm\|add` | Open the popover in that mode on launch. |
| `AKEY_CLOSE_AFTER=<seconds>` | Close the popover after the given delay. |
| `AKEY_SNAPSHOT=/path/out.png` | Save the popover as a PNG, then quit. No Screen Recording permission is needed, but the rings are not captured. |

The app icon ([`resources/AppIcon.icns`](resources/AppIcon.icns)) is generated by `swift scripts/make-icon.swift`. Re-run it after changing the design.

## Uninstall

```sh
akey list | while IFS= read -r n; do akey remove "$n"; done   # delete all accounts from the Keychain
rm -rf /Applications/AKey.app /usr/local/bin/akey
```

## License

[MIT](LICENSE)
