import { defineCachedHandler } from "nitro/cache";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import type { PackageSingleResponse } from "../../../../utils/winget";
import { buildPackageIndex } from "../../../../utils/winget";

/**
 * GET /registry/winget/packages/{PackageIdentifier}
 *
 * WinGet.RestSource API - Get specific package
 *
 * Response: PackageSingleResponse
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

    const response: PackageSingleResponse = {
      PackageIdentifier: packageId,
      Versions: Array.from(versions).sort().reverse(),
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
