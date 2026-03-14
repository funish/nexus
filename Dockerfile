# use the official Bun image
# see all versions at https://hub.docker.com/r/oven/bun/tags
FROM oven/bun:1 AS build
WORKDIR /app

COPY package.json bun.lock* ./

# use ignore-scripts to avoid running postinstall hooks
RUN bun install --frozen-lockfile --ignore-scripts

# Copy the entire project
COPY . .

# Build with node-cluster preset
ENV NITRO_PRESET=node_cluster
ENV NODE_ENV=production
RUN bun --bun run build

# copy production dependencies and source code into final image
FROM oven/bun:1-alpine AS production
WORKDIR /app

# Copy .output directory
COPY --from=build /app/.output /app

# Set cluster workers
ENV NITRO_CLUSTER_WORKERS=max
ENV NODE_ENV=production
ENV PORT=3000

# run the app
EXPOSE 3000/tcp
ENTRYPOINT ["bun", "--bun", "run", "/app/server/index.mjs"]
