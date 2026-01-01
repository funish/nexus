import { defineCachedHandler } from "nitro/cache";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import type { VersionMultipleResponse } from "../../../../../utils/winget";
import { buildPackageIndex } from "../../../../../utils/winget";

/**
 * GET /registry/winget/packages/{PackageIdentifier}/versions
 *
 * WinGet.RestSource API - Get all versions for a package
 *
 * Response: VersionMultipleResponse
 */
export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");

    if (!packageId) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier is required",
      });
    }

    // Build package index
    const packageIndex = await buildPackageIndex();
    const versions = packageIndex.get(packageId);

    if (!versions) {
      throw new HTTPError({
        status: 404,
        statusText: `Package '${packageId}' not found`,
      });
    }

    const response: VersionMultipleResponse = {
      Data: Array.from(versions)
        .sort()
        .reverse()
        .map((version) => ({
          PackageVersion: version,
        })),
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 600,
    swr: true,
    group: "registry:winget",
  },
);
