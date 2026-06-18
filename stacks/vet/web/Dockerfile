# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# vet-web — pnpm build -> nginx static (impl §7). Build context MUST be the monorepo root: the SPA
# consumes the workspace packages @dogtag/ui and @dogtag/standard (workspace:* deps), so the whole
# pnpm workspace is needed to resolve + transpile them. Build with:
#   docker build -f stacks/vet/web/Dockerfile -t dogtag-vet-web .
# ----------------------------------------------------------------------------------------------

# ---- builder ---------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS builder
RUN corepack enable
WORKDIR /app

# Workspace manifests first (better layer caching for the install step).
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages ./packages
COPY stacks/vet/web ./stacks/vet/web
COPY circuits/package.json ./circuits/package.json

# Install across the workspace, then build just the vet portal.
RUN pnpm install --frozen-lockfile --filter @dogtag/vet-web... && \
    pnpm --filter @dogtag/vet-web build

# ---- runtime: nginx serving the static build ------------------------------------------------
FROM nginx:1.27-alpine AS runtime
COPY stacks/vet/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/stacks/vet/web/dist /usr/share/nginx/html
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://localhost/ >/dev/null 2>&1 || exit 1
