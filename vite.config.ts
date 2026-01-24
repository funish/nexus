import { defineConfig } from "vite";
import { nitro } from "nitro/vite";
import pkg from "./package.json";
import { env } from "std-env";

// Check if S3 environment variables are configured
const hasS3Config =
  env.S3_ACCESS_KEY_ID &&
  env.S3_SECRET_ACCESS_KEY &&
  env.S3_ENDPOINT &&
  env.S3_REGION &&
  env.S3_BUCKET;

export default defineConfig({
  plugins: [
    nitro({
      serverDir: "./server/",
      experimental: {
        openAPI: true,
        vite: {
          virtualBundle: true,
          assetsImport: true,
          serverReload: true,
        },
      },
      openAPI: {
        meta: {
          title: pkg.name,
          description: pkg.description,
          version: pkg.version,
        },
        // Enable in production for runtime documentation
        production: "runtime",
        route: "/_docs/openapi.json",
        ui: {
          scalar: {
            route: "/_docs/scalar",
          },
          swagger: {
            route: "/_docs/swagger",
          },
        },
      },
      storage: {
        cache: {
          // Use S3 if configured, otherwise fallback to memory
          driver: hasS3Config ? "s3" : "memory",
          // S3 configuration (only applied when S3 env vars are set)
          ...(hasS3Config && {
            accessKeyId: env.S3_ACCESS_KEY_ID,
            secretAccessKey: env.S3_SECRET_ACCESS_KEY,
            endpoint: env.S3_ENDPOINT,
            region: env.S3_REGION,
            bucket: env.S3_BUCKET,
          }),
        },
      },
      devStorage: {
        cache: {
          driver: "fs",
          base: ".cache",
        },
      },
      compatibilityDate: "2025-12-31",
    }),
  ],
});
