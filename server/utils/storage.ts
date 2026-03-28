import fsDriver from "@bunit/storage/drivers/fs";
import s3Driver from "@bunit/storage/drivers/s3";
import { isDevelopment, env } from "std-env";
import { createStorage } from "unstorage";
import memoryDriver from "unstorage/drivers/memory";

const hasS3Config =
  env.S3_ACCESS_KEY_ID && env.S3_SECRET_ACCESS_KEY && env.S3_ENDPOINT && env.S3_BUCKET;

/**
 * Create storage instances with static drivers
 * - Development: Uses filesystem driver with ./.cache
 * - Production: Uses S3 if S3 env vars are configured
 * - Fallback: Uses memory otherwise
 */

export const cacheStorage = createStorage({
  driver: isDevelopment
    ? fsDriver({
        base: "./.cache",
      })
    : hasS3Config
      ? s3Driver({
          accessKeyId: env.S3_ACCESS_KEY_ID,
          secretAccessKey: env.S3_SECRET_ACCESS_KEY,
          endpoint: env.S3_ENDPOINT,
          region: env.S3_REGION,
          bucket: env.S3_BUCKET,
        })
      : memoryDriver(),
});
