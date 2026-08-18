# Running unsigned macOS desktop builds

Loom's macOS builds are **not code-signed or notarized yet**. Everything below
is a workaround for testing dev and CI builds — not the intended end state for
released software. See [Status](#status).

## What you will see

Opening a `.app` or `.dmg` that Apple has not notarized produces:

> **"Loom" can't be opened because Apple cannot check it for malicious
> software.**

Depending on your macOS version the wording varies ("...is damaged and can't be
opened" also appears, most often for a `.dmg` downloaded through a browser), but
the cause is the same: the download carries a `com.apple.quarantine` extended
attribute, and Gatekeeper refuses to launch quarantined code without a valid
Developer ID signature and notarization ticket.

This is expected for these builds. It is not a sign the download is corrupt.

## Workaround 1 — right-click → Open (GUI)

The one to prefer: it is per-app, and it does not require the terminal.

1. Drag `Loom.app` to `/Applications` (or wherever you keep it).
2. **Right-click** (or Control-click) the app icon → **Open**.
3. A dialog appears with an **Open** button that the normal double-click flow
   does not offer. Click it.

macOS remembers the decision, so subsequent launches work by double-click.

> On macOS 15 (Sequoia) and later, the right-click → Open shortcut has been
> removed for unsigned apps. Instead, double-click once and let it be blocked,
> then go to **System Settings → Privacy & Security**, scroll to the message
> about Loom being blocked, and click **Open Anyway**.

## Workaround 2 — strip the quarantine attribute (terminal)

```sh
xattr -cr /Applications/Loom.app
```

Adjust the path if the app lives elsewhere. `-c` clears all extended attributes
and `-r` recurses into the bundle; afterwards the app launches normally.

Run this only on a build whose origin you trust — you are removing the marker
that tells macOS the file came from the internet, which is exactly the check
that would otherwise protect you. For a build you produced yourself, or one
from this repository's CI, that is a reasonable trade. For a binary from an
unknown source, it is not.

## Status

Proper **code signing and notarization with an Apple Developer certificate is a
known follow-up and is not implemented.** Doing it properly requires a paid
Apple Developer account, a Developer ID Application certificate stored as a CI
secret, and a notarization step in the release workflow that submits the built
app to Apple and staples the returned ticket.

Until then, every macOS artifact this project produces needs one of the
workarounds above.
