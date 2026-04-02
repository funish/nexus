import { cacheStorage } from "../storage";
import type { CdnFile, CdnPackageListing } from "./types";

/**
 * Build a directory listing from cached package metadata.
 *
 * When a file path returns 404, this function checks if the path
 * might be a directory and returns a listing of its contents.
 *
 * @returns CdnPackageListing if directory has contents, or null if empty/invalid
 */
export async function getDirectoryListing(
  cacheBase: string,
  filepath: string,
  packageName: string,
  version: string,
): Promise<CdnPackageListing | null> {
  const storage = cacheStorage;
  const meta = await storage.getMeta(cacheBase);
  const allFiles = (meta?.files || []) as Array<CdnFile>;

  const dirPrefix = `${filepath}/`;
  const dirContents: CdnFile[] = allFiles
    .filter((file) => file.name.startsWith(dirPrefix))
    .map((file) => ({
      name: file.name.slice(dirPrefix.length),
      size: file.size,
      ...(file.integrity ? { integrity: file.integrity } : {}),
    }))
    .filter((file) => file.name.length > 0);

  if (dirContents.length === 0) {
    return null;
  }

  dirContents.sort((a, b) => a.name.localeCompare(b.name));

  return {
    name: packageName,
    version,
    path: filepath,
    files: dirContents,
  };
}
