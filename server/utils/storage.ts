import fsDriver from "@bunit/storage/drivers/fs";
import s3Driver from "@bunit/storage/drivers/s3";
import { isDevelopment, env } from "std-env";
import { createStorage } from "unstorage";
import memoryDriver from "unstorage/drivers/memory";

const hasS3Config =
  env.S3_ACCESS_KEY_ID && env.S3_SECRET_ACCESS_KEY && env.S3_ENDPOINT && env.S3_BUCKET;

/**
 * Filesystem storage (local .cache directory)
 */
export const fsStorage = createStorage({
  driver: fsDriver({
    base: "./.cache",
  }),
});

/**
 * S3 storage (remote, for distributed deployments)
 */
export const s3Storage = hasS3Config
  ? createStorage({
      driver: s3Driver({
        accessKeyId: env.S3_ACCESS_KEY_ID,
        secretAccessKey: env.S3_SECRET_ACCESS_KEY,
        endpoint: env.S3_ENDPOINT,
        region: env.S3_REGION,
        bucket: env.S3_BUCKET,
      }),
    })
  : null;

/**
 * In-memory storage for process-level shared state.
 *
 * Unlike cacheStorage (filesystem/S3), this holds ephemeral runtime state
 * that does not need to survive restarts. Useful for cross-module state
 * sharing without scattered module-level variables.
 *
 * For cluster deployments, consider replacing with a shared-state backend.
 */
export const memoryStorage = createStorage({
  driver: memoryDriver(),
});

/**
 * Cache storage - uses S3 in production (if configured), filesystem otherwise
 */
export const cacheStorage = !isDevelopment && s3Storage ? s3Storage : fsStorage;
