# 0003 — Auth model: VPN-trusted owner, authenticated access for everyone else

- Status: accepted in principle, **not yet implemented**
- Date: 2026-08-18

## Context

Loom is homelab software, so the common case is a single owner reaching it over
a VPN (WireGuard/Tailscale-style) on a network they control. Demanding a full
login flow for that case is friction with little payoff. But Loom is also meant
to be shared — with family, housemates, other admins — and those users may
arrive over any network path, so "it came from the LAN" cannot be the basis for
trust in general.

## Decision

Two paths, one enforcement point:

- **The owner** gets VPN-trusted access. Reaching the backend over the trusted
  network path is sufficient to act as the owner.
- **Every other user** authenticates fully, backed by
  [Authentik](https://goauthentik.io/) as the identity provider.

Authorization is enforced **at the API layer of the Web/backend, regardless of
network path**. Network position may establish *who you are* for the owner; it
never decides *what you may do*. Clients do not enforce anything — a hidden
button is a UX affordance, not a control.

## Consequences

The owner keeps a frictionless path while shared use gets real identities,
without two parallel permission systems: the API is the single choke point, so
one set of checks covers every client and every route. It also means the VPN
becomes part of the security boundary for the owner path, and misconfiguring it
is a real exposure — the backend should therefore be explicit about which
source it considers trusted rather than inferring it loosely.

None of this is built yet. `/health` is currently unauthenticated and the
backend has no notion of a user. This ADR fixes the direction so the auth layer
is not designed from scratch when it is time to write it.
