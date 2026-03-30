import { Database } from "bun:sqlite";

import { type H3Event } from "nitro/h3";

import { getIndexDb } from "./db";
import type { PackageIdentifier, PackageVersion } from "./types";

/**
 * Build package → versions index from index.db (primary source).
 *
 * Uses a simple SQL query: SELECT DISTINCT i.id, v.version FROM manifest m
 * JOIN ids i ON m.id = i.rowid JOIN versions v ON m.version = v.rowid
 *
 * Zero HTTP requests — all data comes from the cached SQLite instance.
 */
export async function buildPackageIndex(
  event?: H3Event,
): Promise<Map<PackageIdentifier, Set<PackageVersion>>> {
  const db = await getIndexDb(event);

  const rows = db
    .query<{ id: string; version: string }, []>(
      "SELECT DISTINCT i.id, v.version FROM manifest m JOIN ids i ON m.id = i.rowid JOIN versions v ON m.version = v.rowid",
    )
    .all();

  const index = new Map<PackageIdentifier, Set<PackageVersion>>();
  for (const row of rows) {
    let versions = index.get(row.id);
    if (!versions) {
      versions = new Set();
      index.set(row.id, versions);
    }
    versions.add(row.version);
  }

  return index;
}

/**
 * Check if a package exists in the index.
 */
export async function packageExists(db: Database, packageId: string): Promise<boolean> {
  const row = db
    .query<{ count: number }, [string]>(
      "SELECT COUNT(*) as count FROM manifest m JOIN ids i ON m.id = i.rowid WHERE i.id = ?1",
    )
    .get(packageId);
  return (row?.count ?? 0) > 0;
}

/**
 * Get all versions for a specific package from index.db.
 */
export async function getPackageVersions(
  db: Database,
  packageId: string,
): Promise<Set<PackageVersion>> {
  const rows = db
    .query<{ version: string }, [string]>(
      "SELECT DISTINCT v.version FROM manifest m JOIN ids i ON m.id = i.rowid JOIN versions v ON m.version = v.rowid WHERE i.id = ?1",
    )
    .all(packageId);

  return new Set(rows.map((r) => r.version));
}
