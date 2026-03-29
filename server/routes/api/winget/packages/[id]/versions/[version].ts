import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type { VersionSingleResponse, VersionSchema } from "../../../../../../utils/winget";
import {
  getVersionManifests,
  fetchManifestContent,
  createWinGetError,
} from "../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Versions", "Get"],
    summary: "Get specific version of a WinGet package",
    description: "Retrieve version metadata for a specific package version",
    parameters: [
      {
        in: "path",
        name: "id",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
      {
        in: "path",
        name: "version",
        description: "Package version",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Successful response with version metadata",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "object",
                  properties: {
                    PackageVersion: { type: "string" },
                    DefaultLocale: { type: "string" },
                    Channel: { type: "string" },
                  },
                  required: ["PackageVersion", "DefaultLocale"],
                },
              },
              required: ["Data"],
            },
          },
        },
      },
      404: {
        description: "Package or version not found",
      },
    },
  },
});

/**
 * GET /api/winget/packages/{PackageIdentifier}/versions/{PackageVersion}
 *
 * WinGet.RestSource API - Get specific version metadata
 *
 * Response: VersionSingleResponse (WinGet 1.9.0)
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");

  if (!packageId || !version) {
    return createWinGetError(event, 400, "PackageIdentifier and PackageVersion are required");
  }

  // Get all manifest files for this version
  const manifestFiles = getVersionManifests(packageId, version);

  if (manifestFiles.length === 0) {
    return createWinGetError(event, 404, `Version ${version} of package '${packageId}' not found`);
  }

  // Build version metadata
  const versionData: VersionSchema = {
    PackageVersion: version,
    DefaultLocale: "en-US", // Fallback default
  };

  // Parse version manifest to get DefaultLocale and Channel
  const versionManifestPath = manifestFiles.find(
    (path) => path.split("/").pop() === `${packageId}.yaml`,
  );

  if (versionManifestPath) {
    try {
      const content = await fetchManifestContent(versionManifestPath);
      const manifest = parseYAML(content) as Record<string, any>;
      versionData.DefaultLocale = manifest.DefaultLocale || "en-US";
      versionData.Channel = manifest.Channel;
    } catch {
      // Keep fallback values on error
    }
  }

  const response: VersionSingleResponse = {
    Data: versionData,
  };
  return response;
});
