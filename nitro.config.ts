import { defineConfig } from "nitro/config";

export default defineConfig({
  serverDir: "./server/",
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
});
