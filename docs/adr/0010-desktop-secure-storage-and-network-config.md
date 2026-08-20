# 0010 — Desktop secure storage and runtime network configuration

- Status: accepted
- Date: 2026-08-20
- Supersedes: the post-auth CORS follow-up in [0005](./0005-cors-policy.md)

## Context

One Desktop binary must connect to a server chosen after installation. Unlike
web-frontend, it cannot compile a deployment-specific API origin into its
bundle. Its refresh token also outlives a process and must not be written to
localStorage or a plaintext settings file.

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
settings. Changing server clears the old server's token pair before remounting
the shared auth/API providers.

Desktop stores the serialized access/refresh token pair through
`tauri-plugin-keyring-store` with its optional wallet/cryptography feature
disabled. The plugin uses the app identifier as its OS credential-store service
and a stable Loom account name. Stronghold remains the fallback if this plugin
ceases to be maintained or loses a required desktop backend; it is not used
today because an encrypted file whose key lifecycle is unresolved is weaker
operationally than the available native credential stores.

The Tauri CSP allows `connect-src` for both `http:` and `https:` in addition to
the IPC endpoints Tauri needs. This is intentionally broader than a typical
fixed-service app: the destination is user-configured, so a build-time hostname
allowlist cannot express the product requirement. Other resource directives
remain narrow; remote images are allowed because account avatar URLs come from
the chosen Loom server.

The backend CORS policy explicitly allows Tauri's known webview origins:
`tauri://localhost`, `https://tauri.localhost`, and `http://tauri.localhost`.
The normal web deployment stays same-origin through its proxy; localhost web
development and operator-configured browser origins are also allowed. There is
no wildcard. Requests authenticate with explicit Bearer tokens rather than
cookies, so another origin has no ambient credential to ride on, but an
explicit list still limits which browser contexts may read API responses.

## Consequences

- Tokens are stored by the OS credential service, not in Desktop's JSON store.
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
