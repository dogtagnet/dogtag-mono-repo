# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# admin-web — pnpm build -> nginx static (impl §7). Build context MUST be the monorepo root.
#   docker build -f stacks/admin/web/Dockerfile -t dogtag-admin-web .
# ----------------------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS builder
RUN corepack enable
WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages ./packages
COPY stacks/admin/web ./stacks/admin/web
COPY circuits/package.json ./circuits/package.json
RUN pnpm install --frozen-lockfile --filter @dogtag/admin-web... && \
    pnpm --filter @dogtag/admin-web build

FROM nginx:1.27-alpine AS runtime
COPY stacks/admin/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/stacks/admin/web/dist /usr/share/nginx/html
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://localhost/ >/dev/null 2>&1 || exit 1
