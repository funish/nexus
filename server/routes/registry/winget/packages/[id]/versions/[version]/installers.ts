import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseYAML } from "confbox";
import type { InstallerMultipleResponse, InstallerSchema } from "../../../../../../../utils/winget";
import { getVersionManifests, fetchManifestContent } from "../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
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
export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");
    const version = getRouterParam(event, "version");

    if (!packageId || !version) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier and PackageVersion are required",
      });
    }

    // Get all manifest files for this version
    const manifestFiles = await getVersionManifests(packageId, version);

    if (manifestFiles.length === 0) {
      throw new HTTPError({
        status: 404,
        statusText: `Version ${version} of package '${packageId}' not found`,
      });
    }

    // Find installer manifest
    const installerFilename = `${packageId}.installer.yaml`;
    const installerManifestPath = manifestFiles.find(
      (path) => path.split("/").pop() === installerFilename,
    );

    if (!installerManifestPath) {
      throw new HTTPError({
        status: 404,
        statusText: `Installer manifest not found for version ${version} of package '${packageId}'`,
      });
    }

    try {
      const content = await fetchManifestContent(installerManifestPath);
      const manifest = parseYAML(content) as Record<string, any>;

      // Extract installers from manifest
      const installers: InstallerSchema[] = [];

      if (manifest.Installers && Array.isArray(manifest.Installers)) {
        for (const installer of manifest.Installers) {
          installers.push({
            InstallerIdentifier: installer.InstallerIdentifier,
            InstallerType: installer.InstallerType,
            InstallerUrl: installer.InstallerUrl,
            Architecture: installer.Architecture,
            Scope: installer.Scope,
            Language: installer.Language,
          });
        }
      }

      const response: InstallerMultipleResponse = {
        Data: installers,
      };

      event.res.headers.set("Content-Type", "application/json");

      return response;
    } catch (error) {
      throw new HTTPError({
        status: 500,
        statusText: `Failed to parse installer manifest: ${String(error)}`,
      });
    }
  },
  {
    maxAge: 3600,
  },
);
