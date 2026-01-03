import { defineConfig } from "nitro/config";
import pkg from "./package.json";
import { provider } from "std-env";

// Dynamically select storage driver based on deployment environment
const isCloudflare = provider === "cloudflare_pages" || provider === "cloudflare_workers";

export default defineConfig({
  serverDir: "./server/",
  experimental: {
    openAPI: true,
  },
  openAPI: {
    meta: {
      title: pkg.name,
      description: pkg.description,
      version: pkg.version,
    },
    // Enable in production for runtime documentation
    production: "prerender",
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
      // Use Cloudflare KV in Cloudflare environments, memory otherwise
      driver: isCloudflare ? "cloudflare-kv" : "memory",
      // KV binding configuration (only applied in Cloudflare environments)
      ...(isCloudflare && {
        binding: "CACHE",
      }),
    },
  },
  devStorage: {
    cache: {
      driver: "fs",
      base: ".cache",
    },
  },
  // Cloudflare bindings configuration (only applied in Cloudflare environments)
  ...(isCloudflare && {
    cloudflare: {
      wrangler: {
        kv_namespaces: [
          {
            binding: "CACHE",
            id: process.env.CLOUDFLARE_KV_CACHE_ID || "",
            preview_id: process.env.CLOUDFLARE_KV_CACHE_PREVIEW_ID || "",
          },
        ],
      },
    },
  }),
  compatibilityDate: "2025-12-31",
});
