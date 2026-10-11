# Uninstall

Toolport installs a gateway entry into each AI client's own config file and keeps
its state in a per-user data directory. A clean removal has two parts: disconnect
the clients while Toolport is still installed, then remove the app. Normal uninstall and upgrades keep your data and credentials.

## 1. Disconnect your clients

In **Settings**, choose **Remove Toolport from all clients** while Toolport is
still installed. Review the per-client results and resolve any errors before
removing the app or its data. You can also run `toolport-gateway --disconnect-all`
as your normal user; `--dry-run` lists the affected clients without changing them.
Do not run this command with sudo. Untouched configs return to their exact original
bytes; subsequent unrelated edits are preserved or reported as conflicts.
Turn off **Launch at login** in Settings.

Windows NSIS and native GTK deb/rpm/Arch packages attempt the same operation before
removing their gateway. Failure is logged with recovery instructions and removal
continues. Keep Toolport's data on failure, reinstall it, and retry the action above.
Linux hooks enumerate accounts with `~/.config/Toolport` or `~/.config/Conduit`
and switch to each account without a login shell. For a custom `XDG_CONFIG_HOME`
or `TOOLPORT_DATA_DIR`, use the in-app action or CLI with that environment first.
Upgrades skip the removal action. Windows installers defer while client gateways
are open and name the affected clients: close their MCP sessions and retry, or
cancel to install later. They never force-kill gateways by name.

**AppImage, Tauri .deb, and macOS:** use the in-app action before deleting the app.
AppImage and dragging a macOS app to Trash have no uninstall hook. The native GTK
.deb is distinct from the Tauri .deb. The Homebrew tap is `btsouth/homebrew-toolport`;
its cask also requires the action before uninstall or zap. Homebrew runs uninstall
preflight blocks during upgrades without exposing an upgrade flag, so the cask
uses a manual action to preserve client connections during updates. On macOS the
CLI is `"/Applications/Toolport.app/Contents/MacOS/toolport-gateway" --disconnect-all`
(adjust the app path if needed).

## 2. Remove the app

- **Windows:** Settings > Apps > Installed apps > Toolport > Uninstall, or run
  `winget uninstall Toolport.Toolport`. The NSIS uninstaller removes the app and its
  bundled gateway.
- **macOS:** quit Toolport, then drag `Toolport.app` from Applications to the Trash.
  With the Homebrew cask, run `brew uninstall --cask toolport`.
- **Linux:** use the package manager that installed it:
  - pacman repository: `sudo pacman -R toolport`. See
    [Arch and Omarchy](arch-pacman-repo.md) for removing the repo and key.
  - `.deb`: `sudo apt remove toolport` (or `sudo dpkg -r toolport`).
  - Native GTK `.rpm`: `sudo dnf remove toolport`.
  - AppImage: quit Toolport and delete the `.AppImage` file.

## Optional: remove Toolport data

While Toolport is installed, choose **Remove Toolport data** in Settings. The
confirmation lists the data directory, credentials and startup entries affected,
and the report file in your home directory. Toolport closes before removal.
This is permanent and never runs during an upgrade or normal uninstall.

For the CLI, close Toolport and run as your normal user, without sudo:

```sh
toolport-gateway --remove-data --dry-run
toolport-gateway --remove-data --confirm
```

On macOS, use the gateway in `Toolport.app/Contents/MacOS/` as in step 1.
The CLI prints JSON results. Exit 0 means complete, 1 means leftovers or a
failure, and 2 means confirmation was required. Review the report before
removing the app.

Removal first inventories startup entries and checks sessions without changing clients.
It restores and disconnects clients through the same path as step 1 only after
that preflight passes, then re-checks sessions before deleting credentials or data.
A failed restoration or retained edited Toolport entry keeps recovery data.
Resolve the reported client paths before retrying. Active gateway sessions block
removal: close the listed sessions and retry. Toolport requests a graceful stop
only from authenticated daemons for this data directory.

The action removes Toolport-created entries from the selected data directory,
including registry, logs, caches, migration exports, client backups, encrypted
secrets and published gateway copies in `bin/`. Other files and folders are
preserved and listed in the report. The empty directory may remain.
For a sole detected install with readable ownership, credential removal clears
all Toolport accounts, including shared master keys and accounts for servers
removed earlier. With another detected Toolport, Conduit or development install,
cleanup is limited to the current registry's server accounts. Accounts also used
by that install, shared master keys and unknown accounts are preserved. Unreadable
ownership keeps credentials for retry. A custom data directory also checks the
default location. Shared Team tokens are preserved when another install exists.
Removed Team tokens are deleted locally only; the Teams client has no server-side
revoke endpoint.

To explicitly remove **all installs' credentials** in the shared `conduit-mcp`
service, including orphaned chunks and master keys, use the separate CLI opt-in:

```sh
toolport-gateway --remove-data --all-installs --dry-run
toolport-gateway --remove-data --all-installs --confirm
```

This clears credentials used by other installs too, while deleting only
Toolport-created entries in the selected data folder. It can proceed with a
corrupt registry, using independent client recovery snapshots. Client restoration
failures still retain recovery data. Settings uses the automatic scope above.
With an active encrypted file backend or a truly headless Linux host with no
session bus or runtime directory, removal continues without a keyring. On a
desktop, Toolport first tries the runtime directory's bus. An unavailable bus or
Secret Service provider reports credential cleanup as incomplete and keeps the
registry so retry can find the same accounts. Startup cleanup precedes
credential deletion. Any partially completed cleanup lists removed and failed
items separately and keeps the registry so retry uses the same ownership scope.
Toolport's own launch-at-login entries are removed; changed or unrecognized
entries are preserved and reported. Failed removals list the exact resource
paths, or the credential scope if the OS denied inventory access.

The resource inventory also includes app caches, preferences, saved state and
WebView storage for `com.tsout.conduit`. These are removed for a sole default
install and preserved with an explanation when another install or a custom data
directory is in use. Homebrew zap removes these app storage paths, while keeping
registry/recovery data and credentials. Custom data directories require a valid
Toolport registry regardless of their folder name, except for the explicit
`--all-installs` option when a corrupt registry file exists.

Native client files, servers and installed package binaries are kept. The package manager removes installed binaries in step 2. The action
uses the current data directory, including a custom `TOOLPORT_DATA_DIR`, an
existing `Conduit` directory or the separate `Toolport-dev` directory.

## Keeping Toolport

To move a client off Toolport without uninstalling, use **Disconnect** on that
client only. To keep a client connected but stop sending its tool definitions, set
`TOOLPORT_DISCOVERY=full` on that client's gateway entry; see
[Configuration](configuration.md).
