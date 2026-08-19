# 0008 — Auth: JWT access tokens, database-backed refresh tokens, group permissions

- Status: accepted
- Date: 2026-08-19

## Context

[0003](./0003-auth-model-vpn-vs-external.md) fixed the *direction* — the API is
the single enforcement point, the owner may be trusted by network path, everyone
else authenticates — but not the mechanism. Nothing was built; `dev-stub-auth`
stood in, accepting any credentials, so the clients had a shaped API to develop
against.

Replacing it needs three decisions: how a session is represented, how a password
is stored, and how "what may this person do" is modelled. Each has an obvious
cheap answer that is wrong in a way that only shows up later.

## Decision

### Sessions: a short JWT plus a long opaque token

Two tokens, with different jobs:

- **Access token** — HS256 JWT, 15 minutes, carrying the user's id, username,
  and effective permission grants. Verified by signature alone.
- **Refresh token** — 256 random bits, 7 days, stored as a SHA-256 hash in
  `refresh_tokens`, rotated on every use and revocable.

The alternatives each give up something this pair keeps.

*Pure JWT* means every request is a signature check and no database round trip,
which is the property worth having — but a JWT cannot be withdrawn. Signing out,
disabling an account, or responding to a stolen token all become "wait for it to
expire". Fixing that with a deny-list reintroduces the database lookup the
design existed to avoid.

*Pure opaque tokens* are revocable by construction, but every authenticated
request becomes a database read. For a homelab with a status poll every ten
seconds per client that is real, permanent load in exchange for revocability
that is exercised rarely.

Splitting them puts the frequent operation on the cheap path and the rare one on
the authoritative path. The cost is a bounded staleness window: a permission
revoked now remains in an already-issued access token until it expires. Fifteen
minutes is the size of that window, and it is why the number is small.

**Rotation** is what makes a 7-day credential defensible. Each refresh issues a
new token and revokes the one presented, so a stolen refresh token is usable at
most once — and its use is *detectable*, because the legitimate holder's next
refresh fails against a token that was already spent.

Refresh tokens are hashed at rest with SHA-256, not argon2. They are already
high-entropy random values, so there is no dictionary to slow down; a slow hash
would only add latency to every refresh.

### Passwords: argon2id

Argon2id at the `argon2` crate's default parameters, which track the OWASP
recommendation rather than a number chosen here. The PHC-format output carries
its parameters, so raising the cost later does not invalidate stored hashes:
they keep verifying under the parameters they were created with and can be
upgraded on next successful login.

The minimum length is 8 characters with no composition rules. Length is what
costs an attacker; character-class rules mostly produce predictable
substitutions. This is a starting point — checking against a breached-password
list would do more than raising the number.

### Permissions: groups, with optional per-resource scope

Permissions are granted to **groups**, never directly to users, so "what can
this person do" always resolves along one path. A grant is a permission key plus
an optional scope:

| `resource_type` | `resource_id` | Meaning |
| --- | --- | --- |
| NULL | NULL | Every resource, of every type |
| set | NULL | Every resource of that type |
| set | set | Exactly that one resource |

Flat roles were the alternative and are simpler right up until the first request
that a housemate may restart the media server but not the router. A flat model
expresses that only by inventing a permission key per resource, which turns
every new connector into a schema change and leaves the key list unbounded.
Scoping the grant instead keeps the key set small and fixed while letting the
*grant* be as narrow as needed. That request is not hypothetical: 0003 exists
because Loom is meant to be shared with family and housemates.

The permission set itself is a table, seeded and extended by migration. A key
used in code must exist there, enforced by a foreign key, so a typo is a failed
migration rather than a grant that silently matches nothing.

### Zero-config

The JWT signing secret is generated from the OS CSPRNG on first boot and stored
in `server_config`, never supplied by environment variable — [0004](./0004-zero-config-startup.md)
forbids required configuration. It has to be persisted rather than regenerated
because a secret that changed on restart would invalidate every outstanding
access token on every deploy.

The SQLite database defaults to `$LOOM_DATA_DIR/loom.db`, falling back to
`./data/loom.db` when the variable is unset, creating the directory if needed.

## Consequences

`dev-stub-auth` is **deleted**, along with its Cargo feature, its CI comment,
and the rule about it in `AGENT_INSTRUCTIONS.md`. The endpoint shapes it
established survive; what changed is that they are now backed by real logic. The
login response is the one breaking change: a single `token` becomes
`accessToken` + `refreshToken`.

Clients now have real work to do that the stub let them skip. They must store
two tokens rather than one, refresh before expiry or on a 401, and handle a
failed refresh by returning to login. Fifteen minutes is short enough that a
client which ignores refresh will appear to sign the user out constantly.

**This is authentication only.** Nothing enforces the permissions yet: the
grants are computed, stored, and delivered in claims, but no middleware consults
them, so the connector routes remain reachable by anyone who can reach the port
— exactly as they were under the stub. The enforcement middleware and the
user/group management endpoints are the deliberate next step. Until they land,
this instance is not access-controlled, and 0003's "the API is the single
enforcement point" is a design statement rather than a description of the code.

Session revocation is coarse: logging out revokes one refresh token, so other
devices stay signed in. There is no "sign out everywhere", and no cleanup job
for expired refresh-token rows yet.

HS256 is symmetric, which is right for one backend instance verifying its own
tokens. If Loom ever grows a second service that must verify tokens without
being able to mint them, this becomes RS256 and the secret becomes a keypair.

SQLite is the store. It suits a single-process homelab server and needs no
second container, but it is one writer at a time and lives on one filesystem —
the constraint to revisit if Loom ever runs more than one backend.
