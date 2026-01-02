import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseYAML } from "confbox";
import type {
  InstallerSingleResponse,
  InstallerSchema,
} from "../../../../../../../../utils/winget";
import { getVersionManifests, fetchManifestContent } from "../../../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
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
export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");
    const version = getRouterParam(event, "version");
    const installerId = getRouterParam(event, "installer");

    if (!packageId || !version || !installerId) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier, PackageVersion, and InstallerIdentifier are required",
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

      // Find the specific installer
      if (!manifest.Installers || !Array.isArray(manifest.Installers)) {
        throw new HTTPError({
          status: 404,
          statusText: `No installers found in manifest`,
        });
      }

      const installer = manifest.Installers.find(
        (inst: any) => inst.InstallerIdentifier === installerId,
      );

      if (!installer) {
        throw new HTTPError({
          status: 404,
          statusText: `Installer '${installerId}' not found`,
        });
      }

      const installerData: InstallerSchema = {
        InstallerIdentifier: installer.InstallerIdentifier,
        InstallerType: installer.InstallerType,
        InstallerUrl: installer.InstallerUrl,
        Architecture: installer.Architecture,
        Scope: installer.Scope,
        Language: installer.Language,
      };

      const response: InstallerSingleResponse = {
        Data: installerData,
      };

      event.res.headers.set("Content-Type", "application/json");

      return response;
    } catch (error) {
      if (error instanceof HTTPError) {
        throw error;
      }
      throw new HTTPError({
        status: 500,
        statusText: `Failed to parse installer manifest: ${String(error)}`,
      });
    }
  },
  {
    maxAge: 3600,
    group: "registry:winget",
  },
);
