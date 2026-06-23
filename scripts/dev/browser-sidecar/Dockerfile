# Playwright base image ships Chromium + system deps matching the playwright
# npm version below. Keep the tag in sync with package.json's playwright range.
FROM mcr.microsoft.com/playwright:v1.48.0-jammy

WORKDIR /app
COPY package.json ./
RUN npm install --omit=dev
COPY server.js ./

ENV PORT=9223
EXPOSE 9223
CMD ["node", "server.js"]
