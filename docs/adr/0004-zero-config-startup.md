# 0004 — Zero-config startup

- Status: accepted in principle, **not yet implemented**
- Date: 2026-08-18

## Context

Loom is self-hosted software aimed at people running a homelab, not at people
who are necessarily fluent in Docker. The current `docker-compose.yml` required
a hand-edited `.env` before it would even parse — `docker compose up` failed
with a variable-interpolation error rather than starting anything. That is a
poor first impression, and it is a self-inflicted one.

The comparable projects set the expectation here: Nextcloud, Immich, and Gitea
all boot from an unmodified Compose file and finish configuration in a browser
on first visit. A new user should be able to copy a Compose file, run
`docker compose up`, open a browser, and be guided from there. Every mandatory
variable before that point is a step where someone gives up.

## Decision

**web-backend starts with zero required environment variables.** Bind address
and log level have working defaults; nothing must be supplied for the process
to come up.

**Secrets are generated, not supplied.** Anything the backend needs but cannot
default — session keys, signing keys — is generated on first boot when not
explicitly provided, and persisted to a mounted data directory so it survives
container restart and recreation. A regenerated signing key would invalidate
every existing session, so persistence is part of the decision, not an
implementation detail. An explicitly supplied value always wins, for operators
who manage secrets externally.

**Initial setup happens in a first-run web UI, not a CLI prompt.** Docker
deployments are routinely non-interactive — `-d`, Compose, TrueNAS Apps,
Portainer, Kubernetes — and in all of those an interactive terminal wizard is
either invisible or unreachable. A browser is the one interface every deployment
path has.

**The backend exposes its setup-completion state**, so any client — web
frontend, desktop, mobile — can detect "setup required" and redirect to the
setup flow rather than each client inventing its own probe. This follows
[0003](./0003-auth-model-vpn-vs-external.md): the backend remains the single
source of truth, and clients only reflect what it reports.

**Deployment-selection config is a separate concern.** Which published image and
tag to pull (`LOOM_IMAGE_OWNER`, `LOOM_WEB_BACKEND_TAG`) is chosen at deployment
time, before any Loom code runs, and is not part of application config. Those
variables may exist and may have their own defaults; this ADR governs
application runtime config, not registry coordinates.

## Consequences

**web-backend needs a persisted data directory from day one of its real config
system.** Both Compose files therefore already declare a `loom-data` volume
mounted at `/data` with `LOOM_DATA_DIR=/data` — the volume exists before the
code that uses it, so deployments created today do not silently lose generated
secrets when that code lands.

**The setup flow becomes a required first screen.** web-frontend needs a setup
route and a redirect when the backend reports setup as incomplete; desktop and
mobile each need the same first-run check. That is real UI work, and it is a
prerequisite for the auth system rather than a polish item afterwards.

**Generated-by-default secrets shift a security property.** A deployment that
never sets a signing key gets a good random one, which is strictly better than
the shipped-default-password pattern; but the data volume now holds material
that matters, and backup and file-permission guidance has to say so.

**None of this is built.** web-backend currently serves a stub `/health` route
and has no config, auth, or persistence layer to attach any of it to.
Implementing it now would mean designing a secret-management system against a
route that needs no secrets. It is deferred until initial auth and
access-management work begins — a known follow-up, recorded here so the
constraint is already settled when that work starts.
