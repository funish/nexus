import { Database } from "bun:sqlite";

import { unzipSync } from "fflate";
import { type H3Event } from "nitro/h3";

import { cacheStorage } from "../storage";
import { INDEX_DB_KEY, SOURCE_MSIX_URL, UPDATE_INTERVAL } from "./constants";

// index.db management

/** Cached Database instance */
let cachedDb: Database | null = null;
/** Timestamp when cachedDb was loaded */
let cachedDbTime = 0;

/**
 * Create missing performance indexes on map tables.
 * tags_map and commands_map lack manifest indexes in the source index.db,
 * which causes NOT EXISTS subqueries to do full table scans.
 */
export function createMissingIndexes(db: Database): void {
  db.run("CREATE INDEX IF NOT EXISTS tags_map_manifest_idx ON tags_map(manifest)");
  db.run("CREATE INDEX IF NOT EXISTS commands_map_manifest_idx ON commands_map(manifest)");
}

/**
 * Download source.msix and extract Public/index.db into cacheStorage,
 * then update the in-memory cached instance
 */
async function refreshIndexDb(): Promise<void> {
  const response = await fetch(SOURCE_MSIX_URL);
  if (!response.ok) {
    throw new Error(`Failed to download source.msix: ${response.status}`);
  }

  const arrayBuffer = await response.arrayBuffer();
  const files = unzipSync(new Uint8Array(arrayBuffer));

  const indexDb = files["Public/index.db"];
  if (!indexDb) {
    throw new Error("index.db not found in source.msix");
  }

  await cacheStorage.setItemRaw(INDEX_DB_KEY, indexDb);
  await cacheStorage.setMeta(INDEX_DB_KEY, { mtime: new Date() });

  // Replace in-memory instance
  cachedDb?.close();
  cachedDb = Database.deserialize(indexDb);
  createMissingIndexes(cachedDb);
  cachedDbTime = Date.now();
}

/**
 * Get index.db as an in-memory SQLite Database instance.
 * Returns the cached instance if fresh, otherwise refreshes.
 */
export async function getIndexDb(event?: H3Event): Promise<Database> {
  const age = cachedDb ? (Date.now() - cachedDbTime) / 1000 : Infinity;

  if (cachedDb && age < UPDATE_INTERVAL) {
    createMissingIndexes(cachedDb);
    return cachedDb;
  }

  // Try to load from cacheStorage (e.g. across restarts)
  if (!cachedDb) {
    const data = await cacheStorage.getItemRaw(INDEX_DB_KEY);
    if (data) {
      const meta = await cacheStorage.getMeta(INDEX_DB_KEY);
      cachedDb = Database.deserialize(new Uint8Array(data as ArrayBuffer));
      createMissingIndexes(cachedDb);
      cachedDbTime = meta?.mtime ? new Date(meta.mtime).getTime() : Date.now();

      if ((Date.now() - cachedDbTime) / 1000 < UPDATE_INTERVAL) {
        return cachedDb;
      }
    }
  }

  // Stale — return old instance and trigger background refresh
  if (cachedDb && event) {
    event.waitUntil(
      (async () => {
        try {
          await refreshIndexDb();
        } catch (error) {
          console.error("Failed to refresh index.db in background:", error);
        }
      })(),
    );
    return cachedDb;
  }

  // Missing or no event — download synchronously
  await refreshIndexDb();
  return cachedDb!;
}
