import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import { getIndexDb } from "../../../../../../utils/winget/db";
import {
  constructManifestPath,
  fetchManifestContent,
} from "../../../../../../utils/winget/manifest";
import { getPackageVersions } from "../../../../../../utils/winget/queries";
import { createWinGetError } from "../../../../../../utils/winget/response";
import type {
  WinGetVersionSingleResponse,
  WinGetVersionSchema,
} from "../../../../../../utils/winget/types";

defineRouteMeta({
  openAPI: {
    tags: ["Versions", "Get"],
    summary: "Get Version Metadata",
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
    ],
    responses: {
      200: {
        description: "Version metadata",
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
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}
 *
 * WinGet.RestSource API - Get specific version metadata
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");

  if (!packageId || !version) {
    return createWinGetError(event, 400, "PackageIdentifier and PackageVersion are required");
  }

  const db = await getIndexDb(event);
  const versions = getPackageVersions(db, packageId);

  if (!versions.has(version)) {
    return createWinGetError(event, 404, `Version ${version} of package '${packageId}' not found`);
  }

  const versionData = {
    PackageVersion: version,
  } as WinGetVersionSchema;

  try {
    const mainPath = constructManifestPath(packageId, version, "main");
    const content = await fetchManifestContent(mainPath);
    const manifest = parseYAML(content) as Record<string, any>;
    Object.assign(versionData, manifest);
  } catch {
    // Keep base values on error
  }

  const response: WinGetVersionSingleResponse = {
    Data: versionData,
  };
  return response;
});
