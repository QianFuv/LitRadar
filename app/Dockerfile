FROM node:24-alpine AS deps

WORKDIR /app

COPY package.json pnpm-lock.yaml ./

RUN corepack enable pnpm && pnpm install --frozen-lockfile


FROM node:24-alpine AS build

WORKDIR /app

COPY --from=deps /app/node_modules node_modules/
COPY . .

ARG INTERNAL_API_URL=http://api:8000
ENV INTERNAL_API_URL=$INTERNAL_API_URL

RUN corepack enable pnpm && pnpm build


FROM node:24-alpine

WORKDIR /app

ENV NODE_ENV=production
ENV HOME=/tmp

COPY --chown=node:node --from=build /app/.next/standalone ./
COPY --chown=node:node --from=build /app/.next/static .next/static/

USER node

EXPOSE 3000

CMD ["node", "server.js"]
