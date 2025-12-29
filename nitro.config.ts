import { defineConfig } from "nitro/config";

export default defineConfig({
  serverDir: "./server/",

  routeRules: {
    "/cdn/**": {
      cache: {
        maxAge: 60 * 60 * 24 * 30,
      },
      headers: {
        "Cache-Control": "public, max-age=31536000, immutable",
        "X-Content-Type-Options": "nosniff",
      },
    },
  },
});
