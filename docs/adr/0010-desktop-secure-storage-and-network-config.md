# 0010 — Installed-client secure storage and runtime network configuration

- Status: accepted
- Date: 2026-08-20
- Supersedes: the post-auth CORS follow-up in [0005](./0005-cors-policy.md)

## Context

One Desktop or Mobile binary must connect to a server chosen after
installation. Unlike web-frontend, it cannot compile a deployment-specific API
origin into its bundle. Its refresh token also outlives a process and must not
be written to localStorage or a plaintext settings file.

Tauri 2 has an official Store plugin for non-sensitive JSON settings and an
official Stronghold plugin for encrypted vault files. Stronghold requires a
user-supplied vault password or another secure place to keep its encryption
key. In August 2026, `tauri-plugin-keyring-store` 0.2.0 is an actively released
Tauri 2 plugin that maps records directly onto macOS Keychain, Windows
Credential Manager, and Linux Secret Service without a plaintext snapshot. It
therefore meets this use case without inventing a master-password lifecycle.

The CSP cannot name the backend host at build time. Homelab servers may also be
HTTP-only, so HTTPS-only network policy would make otherwise valid deployments
unreachable.

## Decision

Desktop stores the server base URL in `tauri-plugin-store`. First launch shows
`ConnectToServer`, validates an HTTP(S) URL, and requires a successful `/health`
response before persisting it. The same component is embedded in General
settings. The stored connection also carries an off-by-default option to accept
an invalid TLS certificate chain for homelab servers using self-signed
certificates. Enabling it does not disable hostname verification. Changing
server clears the old server's token pair before remounting the shared auth/API
providers; changing only this TLS option remounts the API transport without
signing the user out.

Desktop sends Loom API traffic through the official Tauri HTTP plugin rather
than the webview's `fetch`. This gives the platform adapter a native TLS policy,
avoids relying on browser-specific CORS behavior, and lets the shared API client
keep using an injected `HttpTransport`. The plugin's `dangerous-settings`
feature is enabled solely to express the explicit per-server certificate
option. HTTP and HTTPS are both permitted because the server destination is
selected at runtime; no other schemes are in scope.

Connector status WebSockets follow the same ownership boundary through an
injected `WebSocketTransport`. Web supplies a thin browser-WebSocket adapter;
Desktop supplies the official `tauri-plugin-websocket` client and grants its
default connect/send capability. This keeps socket construction out of the
shared connector-status client and gives each installed platform one place to
apply future native network policy.

`@tauri-apps/plugin-websocket` 2.4.2 does not expose an
accept-invalid-certificates option in its JavaScript `ConnectionConfig`. Its
Rust builder accepts a process-wide TLS connector at application startup, but
that cannot follow Loom's runtime, per-server Store setting. Consequently
Desktop supports plain `ws:` and normally validated `wss:`, but the existing
HTTP certificate exception does not extend to self-signed WSS. Initial data and
actions continue over the native HTTP transport; only live status push is
unavailable in that configuration. The connect form states this limitation and
the Desktop adapter is the intended place to add support if the upstream plugin
gains a per-connection policy.

Desktop stores the serialized access/refresh token pair through
`tauri-plugin-keyring-store` with its optional wallet/cryptography feature
disabled. The plugin uses the app identifier as its OS credential-store service
and a stable Loom account name. Stronghold remains the fallback if this plugin
ceases to be maintained or loses a required desktop backend; it is not used
today because an encrypted file whose key lifecycle is unresolved is weaker
operationally than the available native credential stores.

Mobile uses the same `ConnectToServer`, auth provider, API client, and settings
panels from `@loom/ui-kit`. Its native Tauri HTTP adapter supports the same
off-by-default invalid-certificate option as Desktop and persists that
non-sensitive per-server choice with `tauri-plugin-store`. Mobile's Android
WebView WebSocket still requires a normally trusted WSS certificate, so the UI
states that the exception covers API requests but not live status push. The
token pair is stored only in an encrypted Stronghold snapshot. A random
per-install vault password is kept in the app's private Store data, while
Stronghold's Argon2 setup uses an app-local salt. This prevents tokens appearing
as plaintext settings and protects backups or copies of the vault by itself. It
is not equivalent to Desktop's OS credential store against a compromise that
can read all of the app's private data; wrapping the vault password with Android
Keystore is a possible future hardening step.

Android applies a tracked Network Security Configuration to its generated
project. It permits cleartext HTTP for explicitly selected private-network
servers and trusts both system CAs and user-installed CAs. The latter supports
homelab certificate authorities without disabling TLS or hostname validation.
Because `src-tauri/gen` is intentionally ignored, a repository script reapplies
the policy after `tauri android init` in both local builds and CI.

The Tauri CSP allows `connect-src` for `http:`, `https:`, `ws:`, and `wss:` in
addition to the IPC endpoints Tauri needs. This is intentionally broader than a
typical fixed-service app: the destination is user-configured, so a build-time
hostname allowlist cannot express the product requirement. Other resource
directives remain narrow; remote images are allowed because account avatar URLs
come from the chosen Loom server.

The backend CORS policy explicitly allows Tauri's known webview origins:
`tauri://localhost`, `https://tauri.localhost`, and `http://tauri.localhost`
(Android's default mapped origin).
The normal web deployment stays same-origin through its proxy; localhost web
development and operator-configured browser origins are also allowed. There is
no wildcard. Requests authenticate with explicit Bearer tokens rather than
cookies, so another origin has no ambient credential to ride on, but an
explicit list still limits which browser contexts may read API responses.

## Consequences

- Desktop tokens are stored by the OS credential service, not in its JSON
  store. Mobile tokens are stored in Stronghold, not in its JSON settings.
- Certificate verification for HTTP API traffic remains enabled unless the user
  opts out for the selected HTTPS server. The exception accepts more than
  self-signed roots (for example, an expired certificate), so the UI warns that
  it is appropriate only for a server the user trusts; hostname mismatch remains
  an error. WebSocket TLS verification cannot currently opt out, so self-signed
  WSS has no live status push even while API requests continue to work.
- The native HTTP capability allows arbitrary HTTP(S) hosts because the chosen
  server is not known at build time. A compromised trusted webview could use
  that capability, just as it could use the previously broad runtime
  `connect-src`; CSP and Tauri capabilities remain defense-in-depth rather than
  protection from trusted UI code.
- Linux users need an available Secret Service implementation such as GNOME
  Keyring or KWallet. Headless sessions without one cannot persist login state.
- A compromised Desktop webview can still use the native APIs granted to that
  window. CSP and Tauri capabilities reduce exposure but cannot protect secrets
  from code already executing as the trusted UI.
- Allowing both HTTP and HTTPS connections accepts the confidentiality risk of
  user-selected plaintext HTTP. The connect screen does not imply transport
  security; operators should prefer HTTPS whenever their deployment supports it.
- Production browser deployments using a cross-origin API must list their
  frontend origin in `LOOM_CORS_ALLOWED_ORIGINS`; same-origin deployments need
  no setting.
- Android HTTP support accepts the confidentiality risk of a user-selected
  cleartext server. Trusting a user-installed CA grants that CA the same trust
  the device owner assigned it. When Mobile's explicit certificate exception is
  disabled, Loom rejects untrusted, expired, or hostname-mismatched
  certificates; when enabled, it accepts chain/expiry failures for native HTTP
  while continuing to reject hostname mismatches.
