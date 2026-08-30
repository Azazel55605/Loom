# 0030: Session visibility and login rate limiting

## Context

Loom refresh tokens already represented independently revocable sessions, but
users could neither see nor end them. Administrators likewise had no focused
way to respond to a compromised account short of deactivation or deletion.
Login also had uniform credential errors and dummy-password hashing, but no
bound on repeated guessing.

The existing access-token model verifies a signed JWT without reading the
database. Any session feature must preserve that property unless Loom
deliberately replaces the authentication model.

## Decision

- Each refresh-token row records the direct peer IP and a bounded user-agent
  string at issuance. Existing rows remain valid with null context.
- Access tokens carry an optional `sid` claim naming the refresh-token row that
  issued them. Optional decoding preserves compatibility with tokens issued
  before this change. The claim is the only reliable way to mark the current
  session; matching IP addresses or user-agent strings would conflate devices.
- Any authenticated user may list and revoke their own active sessions. Acting
  on another user requires the existing global `users.manage` permission.
  Revoke-all includes the caller's current refresh session.
- Revocation stops future refreshes. Already-issued access tokens remain valid
  for their existing 15-minute lifetime because normal request authentication
  remains database-free.
- Failed logins are tracked in memory per direct peer IP in a rolling
  15-minute window. Ten failures receive the existing generic credential
  response; later attempts receive 429 with `Retry-After`. A successful login
  clears that peer's failures.
- The limiter deliberately keys by IP, not username. A username-keyed limiter
  would let an unauthenticated attacker lock out any known account. IP limiting
  raises the cost of guessing without adding an account-lockout endpoint.
- Forwarded-address headers are not trusted. Until Loom has explicit trusted
  proxy configuration, the socket peer is the only safe identity available.

## Consequences

Users gain self-service device visibility and sign-out-everywhere, and
administrators can end another account's sessions without deleting it. Stored
token hashes and raw refresh tokens are never exposed.

The in-memory limiter has a working zero-configuration default and naturally
forgets state on restart. It is process-local, so multiple backend replicas do
not share a failure window, and distributed attackers can spread guesses over
multiple source addresses. A reverse proxy appears as one peer and therefore
shares one window among the clients behind it. Solving those limitations needs
trusted-proxy and distributed-state decisions rather than silently trusting
caller-controlled headers or introducing a required external service.

Revoking a session is not instantaneous revocation of its current access token.
The maximum residual authorization window remains 15 minutes, matching the
trade-off recorded by the existing auth model.
