# Next.js 16 production image for the harness-console app.
# Railway: set the service Root Directory to `apps/harness-console` so this
# Dockerfile is the build context (the repo root has no package.json, which is
# why auto-detect mis-reads the monorepo). `next start` binds 0.0.0.0:$PORT.
FROM node:22-slim

WORKDIR /app

# install deps first for layer caching (devDeps are needed by `next build`)
COPY package.json package-lock.json ./
RUN npm ci

# build
COPY . .
RUN npm run build

ENV NODE_ENV=production
# nominal; Railway injects $PORT and `next start` honors it
EXPOSE 3000
CMD ["npm", "run", "start"]
