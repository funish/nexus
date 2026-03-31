import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, getRouterParam } from "nitro/h3";

import {
  getVersionManifests,
  fetchManifestContent,
} from "../../../../../../../utils/winget/manifest";
import { createWinGetError } from "../../../../../../../utils/winget/response";
import type {
  InstallerMultipleResponse,
  InstallerSchema,
} from "../../../../../../../utils/winget/types";

defineRouteMeta({
  openAPI: {
    tags: ["Installers", "Get"],
    summary: "Get Installer Metadata",
    parameters: [
      {
        in: "header",
        name: "Version",
        description: "API version",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "header",
        name: "Windows-Package-Manager",
        description: "Windows Package Manager client version",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "path",
        name: "PackageIdentifier",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
      {
        in: "path",
        name: "PackageVersion",
        description: "Package version",
        required: true,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "ContinuationToken",
        description: "Pagination token",
        required: false,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Installer list",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "array",
                  maxItems: 1024,
                  items: {
                    type: "object",
                    properties: {
                      InstallerIdentifier: { type: "string" },
                      InstallerSha256: { type: "string" },
                      InstallerUrl: { type: "string" },
                      Architecture: {
                        type: "string",
                        enum: ["x86", "x64", "arm", "arm64", "neutral"],
                      },
                      InstallerLocale: { type: "string" },
                      Platform: {
                        type: "array",
                        items: { type: "string", enum: ["Windows.Desktop", "Windows.Universal"] },
                      },
                      MinimumOSVersion: { type: "string" },
                      InstallerType: { type: "string" },
                      Scope: { type: "string", enum: ["user", "machine"] },
                      SignatureSha256: { type: "string" },
                      InstallModes: {
                        type: "array",
                        items: {
                          type: "string",
                          enum: ["interactive", "silent", "silentWithProgress"],
                        },
                      },
                      InstallerSwitches: { type: "object" },
                      InstallerSuccessCodes: { type: "array", items: { type: "integer" } },
                      ExpectedReturnCodes: { type: "array", items: { type: "object" } },
                      UpgradeBehavior: {
                        type: "string",
                        enum: ["install", "uninstallPrevious", "deny"],
                      },
                      Commands: { type: "array", items: { type: "string" } },
                      Protocols: { type: "array", items: { type: "string" } },
                      FileExtensions: { type: "array", items: { type: "string" } },
                      Dependencies: { type: "object" },
                      PackageFamilyName: { type: "string" },
                      ProductCode: { type: "string" },
                      Capabilities: { type: "array", items: { type: "string" } },
                      RestrictedCapabilities: { type: "array", items: { type: "string" } },
                      MSStoreProductIdentifier: { type: "string" },
                      InstallerAbortsTerminal: { type: "boolean" },
                      ReleaseDate: { type: "string", format: "date" },
                      InstallLocationRequired: { type: "boolean" },
                      RequireExplicitUpgrade: { type: "boolean" },
                      ElevationRequirement: {
                        type: "string",
                        enum: ["elevationRequired", "elevationProhibited", "elevatesSelf"],
                      },
                      UnsupportedOSArchitectures: { type: "array", items: { type: "string" } },
                      AppsAndFeaturesEntries: { type: "array", items: { type: "object" } },
                      Markets: { type: "object" },
                      NestedInstallerType: { type: "string" },
                      NestedInstallerFiles: { type: "array", items: { type: "object" } },
                      DisplayInstallWarnings: { type: "boolean" },
                      UnsupportedArguments: { type: "array", items: { type: "string" } },
                      InstallationMetadata: { type: "object" },
                      DownloadCommandProhibited: { type: "boolean" },
                      RepairBehavior: {
                        type: "string",
                        enum: ["modify", "uninstaller", "installer"],
                      },
                      ArchiveBinariesDependOnPath: { type: "boolean" },
                    },
                    required: ["Architecture", "InstallerType"],
                  },
                },
                ContinuationToken: { type: "string" },
              },
              required: ["Data"],
            },
          },
        },
      },
      404: { description: "Not Found" },
      default: {
        description: "An Error Occurred.",
        content: {
          "application/json": {
            schema: {
              type: "array",
              items: {
                type: "object",
                properties: {
                  ErrorCode: { type: "integer" },
                  ErrorMessage: { type: "string" },
                },
                required: ["ErrorCode", "ErrorMessage"],
              },
            },
          },
        },
      },
    },
  },
});

/**
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/installers
 *
 * WinGet.RestSource API - Get all installers for a version
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");

  if (!packageId || !version) {
    return createWinGetError(event, 400, "PackageIdentifier and PackageVersion are required");
  }

  const query = getQuery(event);
  const continuationToken = query.ContinuationToken as string | undefined;

  const manifestFiles = await getVersionManifests(packageId, version);

  if (manifestFiles.length === 0) {
    return createWinGetError(event, 404, `Version ${version} of package '${packageId}' not found`);
  }

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

    const allInstallers: InstallerSchema[] = [];
    if (manifest.Installers && Array.isArray(manifest.Installers)) {
      for (const installer of manifest.Installers) {
        allInstallers.push({ ...manifest, ...installer } as InstallerSchema);
      }
    }

    let startIndex = 0;
    if (continuationToken) {
      try {
        startIndex = parseInt(Buffer.from(continuationToken, "base64").toString(), 10);
      } catch {
        startIndex = 0;
      }
    }

    const paginatedInstallers = allInstallers.slice(startIndex, startIndex + 25);

    const response: InstallerMultipleResponse = {
      Data: paginatedInstallers,
    };

    if (startIndex + 25 < allInstallers.length) {
      response.ContinuationToken = Buffer.from((startIndex + 25).toString()).toString("base64");
    }

    return response;
  } catch (error) {
    return createWinGetError(event, 500, `Failed to parse installer manifest: ${String(error)}`);
  }
});
