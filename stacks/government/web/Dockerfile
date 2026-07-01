# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# government-web — pnpm build -> nginx static. The government portal is a deliberately lean SPA
# skeleton (no shared @dogtag/ui / wallet stack), but it still builds through the pnpm workspace.
# Build context MUST be the monorepo root:
#   docker build -f stacks/government/web/Dockerfile -t dogtag-government-web .
# ----------------------------------------------------------------------------------------------

# ---- builder ---------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS builder
RUN corepack enable
WORKDIR /app

# Workspace manifests first (better layer caching for the install step).
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages ./packages
COPY stacks/government/web ./stacks/government/web
COPY circuits/package.json ./circuits/package.json

# Install across the workspace, then build just the government portal.
RUN pnpm install --frozen-lockfile --filter @dogtag/government-web... && \
    pnpm --filter @dogtag/government-web build

# ---- runtime: nginx serving the static build ------------------------------------------------
FROM nginx:1.27-alpine AS runtime
COPY stacks/government/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/stacks/government/web/dist /usr/share/nginx/html
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://localhost/ >/dev/null 2>&1 || exit 1
