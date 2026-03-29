import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type { InstallerMultipleResponse, InstallerSchema } from "../../../../../../../utils/winget";
import {
  getVersionManifests,
  fetchManifestContent,
  createWinGetError,
} from "../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Installers", "Get"],
    summary: "Get all installers for a package version",
    description: "Retrieve all available installers for a specific package version",
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
        description: "Successful response with installers list",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "array",
                  items: {
                    type: "object",
                    properties: {
                      InstallerIdentifier: { type: "string" },
                      InstallerType: { type: "string" },
                      InstallerUrl: { type: "string" },
                      Architecture: { type: "string" },
                      Scope: { type: "string" },
                      Language: { type: "string" },
                    },
                  },
                },
                ContinuationToken: { type: "string" },
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
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/installers
 *
 * WinGet.RestSource API - Get all installers for a version
 *
 * Response: InstallerMultipleResponse
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

  // Find installer manifest
  const installerFilename = `${packageId}.installer.yaml`;
  const installerManifestPath = manifestFiles.find(
    (path) => path.split("/").pop() === installerFilename,
  );

  if (!installerManifestPath) {
    return createWinGetError(
      event,
      404,
      `Installer manifest not found for version ${version} of package '${packageId}'`,
    );
  }

  try {
    const content = await fetchManifestContent(installerManifestPath);
    const manifest = parseYAML(content) as Record<string, any>;

    // Extract installers from manifest
    const installers: InstallerSchema[] = [];

    if (manifest.Installers && Array.isArray(manifest.Installers)) {
      for (const installer of manifest.Installers) {
        installers.push(installer as InstallerSchema);
      }
    }

    const response: InstallerMultipleResponse = {
      Data: installers,
    };

    return response;
  } catch (error) {
    return createWinGetError(event, 500, `Failed to parse installer manifest: ${String(error)}`);
  }
});
