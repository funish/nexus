import { defineConfig } from "nitro/config";
import pkg from "./package.json";

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
      driver: "memory",
    },
  },
  devStorage: {
    cache: {
      driver: "fs",
      base: ".cache",
    },
  },
  compatibilityDate: "2025-12-31",
});
