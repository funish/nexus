import { defineCachedHandler } from "nitro/cache";
import { getQuery } from "nitro/h3";
import type { PackageMultipleResponse, WinGetPackage } from "../../../utils/winget";
import { buildPackageIndex } from "../../../utils/winget";

/**
 * GET /registry/winget/packages
 *
 * WinGet.RestSource API - Get all packages
 *
 * Query parameters:
 * - ContinuationToken: pagination token
 *
 * Response: PackageMultipleResponse
 */
export default defineCachedHandler(
  async (event) => {
    const query = getQuery(event);
    const continuationToken = query.ContinuationToken as string | undefined;

    // Build package index from GitHub tree
    const packageIndex = await buildPackageIndex();

    // Convert to array and sort
    const packages: WinGetPackage[] = Array.from(packageIndex.entries())
      .map(([packageId, versions]) => ({
        PackageIdentifier: packageId,
        Versions: Array.from(versions).sort().reverse(),
      }))
      .sort((a, b) => a.PackageIdentifier.localeCompare(b.PackageIdentifier));

    // Simple pagination (100 items per page)
    const pageSize = 100;
    let startIndex = 0;

    if (continuationToken) {
      // Parse continuation token as base64 encoded offset
      try {
        startIndex = parseInt(Buffer.from(continuationToken, "base64").toString(), 10);
      } catch {
        startIndex = 0;
      }
    }

    const endIndex = startIndex + pageSize;
    const paginatedPackages = packages.slice(startIndex, endIndex);

    const response: PackageMultipleResponse = {
      Data: paginatedPackages,
    };

    // Add continuation token if there are more results
    if (endIndex < packages.length) {
      response.ContinuationToken = Buffer.from(endIndex.toString()).toString("base64");
    }

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 600,
    swr: true,
    group: "registry:winget",
  },
);
