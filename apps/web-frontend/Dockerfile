# syntax=docker/dockerfile:1

# Build context is the REPO ROOT (see docker-compose.local.yml). This app is a
# member of the pnpm workspace, so installing needs the root pnpm-lock.yaml and
# pnpm-workspace.yaml — neither of which exists inside apps/web-frontend.
#
# The API is reached SAME-ORIGIN: nginx proxies /api to the backend, so no API
# host is compiled into the bundle and the same image works for every
# deployment. The backend address is a runtime setting (LOOM_BACKEND_ORIGIN),
# not a build argument. See docs/adr/0006-frontend-api-same-origin.md.
#
# VITE_API_URL still exists as a build-time escape hatch for pointing the bundle
# at an absolute backend URL instead; it is unset by default and, when used,
# makes requests cross-origin (and therefore subject to the backend's CORS
# policy).

# Node 22.13+ is required, not optional: pnpm 11 imports `node:sqlite`, which
# does not exist in Node 20, so `pnpm fetch` dies with ERR_UNKNOWN_BUILTIN_MODULE
# there. Keep this in step with the `packageManager` pin in the root package.json.
FROM node:22-slim AS builder

ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable

WORKDIR /build

# Dependency layer. `pnpm fetch` populates the virtual store from the LOCKFILE
# ALONE — it ignores package manifests entirely — so this layer is invalidated
# only when dependencies actually change, and it does not need to know which
# apps/* packages exist. Enumerating each app's package.json here (the obvious
# alternative) silently breaks the moment a new app joins the workspace, since
# the lockfile would reference an importer whose manifest was never copied.
#
# package.json is copied for its `packageManager` pin, so corepack resolves the
# same pnpm version the repo uses; pnpm-workspace.yaml carries the allowBuilds
# approval esbuild needs.
COPY pnpm-lock.yaml package.json pnpm-workspace.yaml ./
RUN pnpm fetch

# Source layer. Changes here re-run the (fast, network-free) link step but never
# the fetch above.
COPY apps ./apps
RUN pnpm install --offline --frozen-lockfile

# Escape hatch only — normally unset, so the bundle uses its relative /api base.
# Passing an empty value is treated as unset by the app (see resolveApiUrl).
ARG VITE_API_URL=""
ENV VITE_API_URL=$VITE_API_URL

RUN pnpm --filter web-frontend build

FROM nginx:alpine AS runtime

# Rendered by the image's entrypoint at container start, substituting
# LOOM_BACKEND_ORIGIN below.
COPY apps/web-frontend/nginx.conf /etc/nginx/templates/default.conf.template
COPY --from=builder /build/apps/web-frontend/dist /usr/share/nginx/html

# Where nginx forwards /api. The default is the Compose service name, so the
# stack works with no configuration at all — see
# docs/adr/0004-zero-config-startup.md. Override it when the backend lives
# elsewhere.
ENV LOOM_BACKEND_ORIGIN=http://web-backend:8080

EXPOSE 80
