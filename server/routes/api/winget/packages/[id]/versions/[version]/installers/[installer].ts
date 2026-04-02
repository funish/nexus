import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import {
  getVersionManifests,
  fetchManifestContent,
} from "../../../../../../../../utils/winget/manifest";
import { createWinGetError } from "../../../../../../../../utils/winget/response";
import type {
  WinGetInstallerSingleResponse,
  WinGetInstallerSchema,
} from "../../../../../../../../utils/winget/types";

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
        in: "path",
        name: "InstallerIdentifier",
        description: "Installer identifier",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Installer metadata",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
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
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/installers/{InstallerIdentifier}
 *
 * WinGet.RestSource API - Get specific installer
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

    if (!manifest.Installers || !Array.isArray(manifest.Installers)) {
      return createWinGetError(event, 404, `No installers found in manifest`);
    }

    const installer = manifest.Installers.find(
      (inst: any) => inst.InstallerIdentifier === installerId,
    );

    if (!installer) {
      return createWinGetError(event, 404, `Installer '${installerId}' not found`);
    }

    const response: WinGetInstallerSingleResponse = {
      Data: { ...manifest, ...installer } as WinGetInstallerSchema,
    };

    return response;
  } catch (error) {
    return createWinGetError(event, 500, `Failed to parse installer manifest: ${String(error)}`);
  }
});
