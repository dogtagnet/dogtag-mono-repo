# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# groomer-web — pnpm build -> nginx static (impl §7). Build context MUST be the monorepo root.
#   docker build -f stacks/groomer/web/Dockerfile -t dogtag-groomer-web .
# ----------------------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS builder
RUN corepack enable
WORKDIR /app
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages ./packages
COPY stacks/groomer/web ./stacks/groomer/web
COPY circuits/package.json ./circuits/package.json
RUN pnpm install --frozen-lockfile --filter @dogtag/groomer-web... && \
    pnpm --filter @dogtag/groomer-web build

FROM nginx:1.27-alpine AS runtime
COPY stacks/groomer/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/stacks/groomer/web/dist /usr/share/nginx/html
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://localhost/ >/dev/null 2>&1 || exit 1
