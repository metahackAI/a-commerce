# syntax=docker/dockerfile:1.7
#
# The web app (Next.js standalone output). Runs as the image's `node` user (uid 1000).
#
#   docker build -f infra/docker/web.Dockerfile -t acom/acom-web:local .
#
# Context: the repository root (see .dockerignore).

ARG NODE_VERSION=24-bookworm-slim

FROM node:${NODE_VERSION} AS deps
WORKDIR /app
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund

FROM node:${NODE_VERSION} AS build
WORKDIR /app
ENV NEXT_TELEMETRY_DISABLED=1
COPY --from=deps /app/node_modules ./node_modules
COPY web/ ./
RUN npm run build

FROM node:${NODE_VERSION} AS runtime
LABEL org.opencontainers.image.title="acom-web" \
      org.opencontainers.image.source="https://github.com/metahackAI/a-commerce" \
      org.opencontainers.image.licenses="Apache-2.0"
WORKDIR /app
ENV NODE_ENV=production NEXT_TELEMETRY_DISABLED=1 PORT=3000 HOSTNAME=0.0.0.0
COPY --from=build --chown=node:node /app/.next/standalone ./
COPY --from=build --chown=node:node /app/.next/static ./.next/static
COPY --from=build --chown=node:node /app/public ./public
USER node
EXPOSE 3000
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
  CMD ["node", "-e", "fetch('http://127.0.0.1:3000/healthz').then(r => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"]
CMD ["node", "server.js"]
