import { defineConfig } from "nitro/config";

export default defineConfig({
  serverDir: "./server/",
  storage: {
    cache: {
      // production cache storage
    },
  },
  devStorage: {
    cache: {
      // development cache storage
      driver: "fs",
      base: ".cache",
    },
  },
});
