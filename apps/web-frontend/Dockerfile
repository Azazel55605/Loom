# syntax=docker/dockerfile:1

# Build context is the REPO ROOT (see docker-compose.local.yml). This app is a
# member of the pnpm workspace, so installing needs the root pnpm-lock.yaml and
# pnpm-workspace.yaml — neither of which exists inside apps/web-frontend.
#
# NOTE: VITE_API_URL is baked in at BUILD time, not read at runtime. This is a
# static SPA served by nginx: there is no server-side process to read an
# environment variable per request, so changing the API URL requires rebuilding
# the image. A runtime-configurable API URL (an entrypoint that rewrites a
# placeholder, or a /config.json fetched on boot) is a known follow-up and is
# not solved here.

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

ARG VITE_API_URL
ENV VITE_API_URL=$VITE_API_URL

RUN pnpm --filter web-frontend build

FROM nginx:alpine AS runtime

COPY apps/web-frontend/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /build/apps/web-frontend/dist /usr/share/nginx/html

EXPOSE 80
