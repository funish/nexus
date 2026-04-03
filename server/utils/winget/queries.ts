import { Database } from "bun:sqlite";

import { type H3Event } from "nitro/h3";

import { memoryStorage } from "../storage";
import { WINGET_CACHE_PREFIX } from "./constants";
import { getIndexDb } from "./db";
import type { WinGetPackageIdentifier, WinGetPackageVersion } from "./types";

const PACKAGE_INDEX_KEY = `${WINGET_CACHE_PREFIX}/package-index.json`;

/**
 * Invalidate the package index cache.
 * Called after a successful index.db refresh.
 */
export function invalidatePackageIndex(): void {
  memoryStorage.removeItem(PACKAGE_INDEX_KEY).catch(() => {});
}

/**
 * Get the package → versions index from index.db.
 *
 * Results are cached in memoryStorage and only rebuilt when the underlying
 * index.db is refreshed. Zero HTTP requests — all data comes from the
 * cached SQLite instance.
 */
export async function getPackageIndex(
  event?: H3Event,
): Promise<Map<WinGetPackageIdentifier, Set<WinGetPackageVersion>>> {
  const cached = await memoryStorage.getItem(PACKAGE_INDEX_KEY);
  if (cached) {
    // Deserialize: [id, version[]][] → Map<id, Set<version>>
    const entries = cached as [WinGetPackageIdentifier, WinGetPackageVersion[]][];
    const index = new Map<WinGetPackageIdentifier, Set<WinGetPackageVersion>>();
    for (const [id, versions] of entries) {
      index.set(id, new Set(versions));
    }
    return index;
  }

  const db = await getIndexDb(event);

  const rows = db
    .query<{ id: string; version: string }, []>(
      "SELECT DISTINCT i.id, v.version FROM manifest m JOIN ids i ON m.id = i.rowid JOIN versions v ON m.version = v.rowid",
    )
    .all();

  const index = new Map<WinGetPackageIdentifier, Set<WinGetPackageVersion>>();
  for (const row of rows) {
    let versions = index.get(row.id);
    if (!versions) {
      versions = new Set();
      index.set(row.id, versions);
    }
    versions.add(row.version);
  }

  // Serialize Map to JSON-storable format (Set → array)
  const serializable = [...index.entries()].map(([id, versions]) => [id, [...versions]]);
  await memoryStorage.setItem(PACKAGE_INDEX_KEY, serializable);
  return index;
}

/**
 * Check if a package exists in the index using an efficient EXISTS query.
 */
export function packageExists(db: Database, packageId: string): boolean {
  const row = db
    .query<{ ok: number }, [string]>(
      "SELECT 1 AS ok FROM manifest m JOIN ids i ON m.id = i.rowid WHERE i.id = ?1 LIMIT 1",
    )
    .get(packageId);
  return row !== null;
}

/**
 * Get all versions for a specific package from index.db.
 */
export function getPackageVersions(db: Database, packageId: string): Set<WinGetPackageVersion> {
  const rows = db
    .query<{ version: string }, [string]>(
      "SELECT DISTINCT v.version FROM manifest m JOIN ids i ON m.id = i.rowid JOIN versions v ON m.version = v.rowid WHERE i.id = ?1",
    )
    .all(packageId);

  return new Set(rows.map((r) => r.version));
}
