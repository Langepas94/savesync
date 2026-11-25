# Update delivery

The desktop application checks for a stable release at startup and once every 24 hours while it remains open. The user starts installation from Settings. Update downloads and verification run in the background. The daemon is stopped through its normal shutdown path before files are replaced.

Windows uses the existing Authenticode-verified installer. A signed Windows release requires `SAVESYNC_SIGNER_SHA256` to be embedded at build time, as configured by the release workflow.

SteamOS uses `savesync-steamos.tar.gz` and its SHA-256 checksum. The release workflow signs the exact checksum file using Ed25519 and publishes `savesync-steamos.tar.gz.sha256.sig`. The app verifies that signature using the public key embedded at build time before extracting the archive. It checks the archive version and invokes the included user-level installation script, which stops the daemon, stages files, and rolls back if the daemon fails to start. The app is restarted after the installation attempt, including after rollback. The update is available only for the installation at `~/.local/bin/savesync-app`.

Before creating a release tag, configure a dedicated Ed25519 key pair:

1. Generate a private key in PEM format with `openssl genpkey -algorithm ED25519 -out update-key.pem`. Keep it outside the repository and back it up securely.
2. Set the GitHub Actions secret `SAVESYNC_UPDATE_PRIVATE_KEY_BASE64` to the base64 encoding of the PEM file.
3. Set the repository variable `SAVESYNC_UPDATE_PUBLIC_KEY` to the lowercase hexadecimal encoding of the 32-byte raw public key. For example: `openssl pkey -in update-key.pem -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n'`.

The SteamOS release build embeds the public key, and release signing checks that it matches the private key. Install one build containing the intended public key before expecting in-app updates; older builds cannot gain trust in a new key automatically. Keep the key stable across releases. A missing key disables installation rather than accepting unsigned updates.

The published release must be stable and contain the platform-specific files. Drafts and prereleases are ignored. The public GitHub repository permits update checks and downloads without a token.

## Steam Deck installation from GitHub

Download [SaveSync-SteamDeck.run](https://github.com/Langepas94/savesync/releases/latest/download/SaveSync-SteamDeck.run), allow the file to run in Dolphin, then open it in Desktop Mode. For future updates, select **Settings → Updates → Check for updates** in SaveSync.
