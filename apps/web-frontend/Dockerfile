# syntax=docker/dockerfile:1

# Build context is the REPO ROOT (see docker-compose.local.yml). This app is a
# member of the pnpm workspace, so `pnpm install --frozen-lockfile` needs the
# root pnpm-lock.yaml and pnpm-workspace.yaml — neither of which exists inside
# apps/web-frontend.
#
# NOTE: VITE_API_URL is baked in at BUILD time, not read at runtime. This is a
# static SPA served by nginx: there is no server-side process to read an
# environment variable per request, so changing the API URL requires rebuilding
# the image. A runtime-configurable API URL (an entrypoint that rewrites a
# placeholder, or a /config.json fetched on boot) is a known follow-up and is
# not solved here.

FROM node:20-slim AS builder

ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH
RUN corepack enable

WORKDIR /build

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web-frontend/package.json ./apps/web-frontend/

RUN pnpm install --frozen-lockfile

COPY apps/web-frontend ./apps/web-frontend

ARG VITE_API_URL
ENV VITE_API_URL=$VITE_API_URL

RUN pnpm --filter web-frontend build

FROM nginx:alpine AS runtime

COPY apps/web-frontend/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /build/apps/web-frontend/dist /usr/share/nginx/html

EXPOSE 80
