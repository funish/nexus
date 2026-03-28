import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type {
  InstallerSingleResponse,
  InstallerSchema,
} from "../../../../../../../../utils/winget";
import {
  getVersionManifests,
  fetchManifestContent,
  createWinGetError,
} from "../../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet API"],
    summary: "Get specific installer for a package version",
    description: "Retrieve detailed installer information for a specific package version",
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
      {
        in: "path",
        name: "installer",
        description: "Installer identifier",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Successful response with installer details",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
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
              required: ["Data"],
            },
          },
        },
      },
      404: {
        description: "Package, version, or installer not found",
      },
    },
  },
});

/**
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/installers/{InstallerIdentifier}
 *
 * WinGet.RestSource API - Get specific installer
 *
 * Response: InstallerSingleResponse
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");
  const installerId = getRouterParam(event, "installer");

  if (!packageId || !version || !installerId) {
    return createWinGetError(
      event,
      400,
      "PackageIdentifier, PackageVersion, and InstallerIdentifier are required",
    );
  }

  // Get all manifest files for this version
  const manifestFiles = await getVersionManifests(packageId, version);

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

    // Find the specific installer
    if (!manifest.Installers || !Array.isArray(manifest.Installers)) {
      return createWinGetError(event, 404, `No installers found in manifest`);
    }

    const installer = manifest.Installers.find(
      (inst: any) => inst.InstallerIdentifier === installerId,
    );

    if (!installer) {
      return createWinGetError(event, 404, `Installer '${installerId}' not found`);
    }

    const response: InstallerSingleResponse = {
      Data: installer as InstallerSchema,
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  } catch (error) {
    return createWinGetError(event, 500, `Failed to parse installer manifest: ${String(error)}`);
  }
});
