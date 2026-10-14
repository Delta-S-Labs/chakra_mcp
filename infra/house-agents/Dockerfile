# syntax=docker/dockerfile:1
# chakramcp.com's house agents (Ping and the ChakraMCP Guide). Build from the
# repo root, so the Guide's docs can be bundled in:
#   docker build -f infra/house-agents/Dockerfile -t chakramcp-house-agents .
# Dockerfile.dockerignore (next to this file) limits the context to what's
# copied below.

FROM node:24-slim AS build
WORKDIR /repo/infra/house-agents
COPY infra/house-agents/package.json infra/house-agents/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY infra/house-agents/tsconfig.json infra/house-agents/tsconfig.build.json ./
COPY infra/house-agents/scripts ./scripts
COPY infra/house-agents/src ./src
# The docs the Guide answers from (scripts/bundle-docs.mjs).
COPY README.md /repo/README.md
COPY docs/INSTALL.md /repo/docs/INSTALL.md
COPY docs/self-hosting /repo/docs/self-hosting
COPY frontend/public/llms.txt /repo/frontend/public/llms.txt
COPY .claude/skills/chakramcp-agent/SKILL.md /repo/.claude/skills/chakramcp-agent/SKILL.md
RUN npm run build && npm prune --omit=dev

FROM node:24-slim
ENV NODE_ENV=production
WORKDIR /app
COPY --from=build /repo/infra/house-agents/package.json ./
COPY --from=build /repo/infra/house-agents/node_modules ./node_modules
COPY --from=build /repo/infra/house-agents/dist ./dist
# Sessions and limit counters (SQLite). A named volume mounted here starts
# out owned by `node`.
RUN mkdir -p /data && chown node:node /data
USER node
VOLUME ["/data"]
CMD ["node", "--disable-warning=ExperimentalWarning", "dist/main.js"]
