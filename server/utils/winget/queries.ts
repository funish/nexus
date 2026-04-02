import { Database } from "bun:sqlite";

import { type H3Event } from "nitro/h3";

import { getIndexDb } from "./db";
import type { WinGetPackageIdentifier, WinGetPackageVersion } from "./types";

// Cached package index — rebuilt only when index.db refreshes
let cachedPackageIndex: Map<WinGetPackageIdentifier, Set<WinGetPackageVersion>> | null = null;

/**
 * Invalidate the in-memory package index cache.
 * Called after a successful index.db refresh.
 */
export function invalidatePackageIndex(): void {
  cachedPackageIndex = null;
}

/**
 * Get the package → versions index from index.db.
 *
 * Results are cached in memory and only rebuilt when the underlying
 * index.db is refreshed. Zero HTTP requests — all data comes from the
 * cached SQLite instance.
 */
export async function getPackageIndex(
  event?: H3Event,
): Promise<Map<WinGetPackageIdentifier, Set<WinGetPackageVersion>>> {
  if (cachedPackageIndex) return cachedPackageIndex;

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

  cachedPackageIndex = index;
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
