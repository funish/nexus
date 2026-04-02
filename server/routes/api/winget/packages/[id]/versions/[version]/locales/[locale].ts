import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import {
  getVersionManifests,
  fetchManifestContent,
} from "../../../../../../../../utils/winget/manifest";
import { createWinGetError } from "../../../../../../../../utils/winget/response";
import type {
  WinGetLocaleSingleResponse,
  WinGetLocaleSchema,
} from "../../../../../../../../utils/winget/types";

defineRouteMeta({
  openAPI: {
    tags: ["Locale", "Get"],
    summary: "Get Locale Metadata",
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
        name: "PackageLocale",
        description: "Locale code",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Locale metadata",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "object",
                  properties: {
                    PackageLocale: { type: "string" },
                    Publisher: { type: "string" },
                    PublisherUrl: { type: "string" },
                    PublisherSupportUrl: { type: "string" },
                    PrivacyUrl: { type: "string" },
                    Author: { type: "string" },
                    PackageName: { type: "string" },
                    PackageUrl: { type: "string" },
                    License: { type: "string" },
                    LicenseUrl: { type: "string" },
                    Copyright: { type: "string" },
                    CopyrightUrl: { type: "string" },
                    ShortDescription: { type: "string" },
                    Description: { type: "string" },
                    Tags: { type: "array", items: { type: "string" } },
                    ReleaseNotes: { type: "string" },
                    ReleaseNotesUrl: { type: "string" },
                    Agreements: { type: "array", items: { type: "object" } },
                    PurchaseUrl: { type: "string" },
                    InstallationNotes: { type: "string" },
                    Documentations: { type: "array", items: { type: "object" } },
                    Icons: { type: "array", items: { type: "object" } },
                  },
                  required: ["PackageLocale"],
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
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/locales/{PackageLocale}
 *
 * WinGet.RestSource API - Get specific locale
 */
export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");
  const version = getRouterParam(event, "version");
  const locale = getRouterParam(event, "locale");

  if (!packageId || !version || !locale) {
    return createWinGetError(
      event,
      400,
      "PackageIdentifier, PackageVersion, and PackageLocale are required",
    );
  }

  const manifestFiles = await getVersionManifests(packageId, version);

  if (manifestFiles.length === 0) {
    return createWinGetError(event, 404, `Version ${version} of package '${packageId}' not found`);
  }

  const localeFilename = `${packageId}.locale.${locale}.yaml`;
  const localeManifestPath = manifestFiles.find((path) => path.split("/").pop() === localeFilename);

  if (!localeManifestPath) {
    return createWinGetError(
      event,
      404,
      `Locale '${locale}' not found for version ${version} of package '${packageId}'`,
    );
  }

  try {
    const content = await fetchManifestContent(localeManifestPath);
    const manifest = parseYAML(content) as Record<string, any>;

    const response: WinGetLocaleSingleResponse = {
      Data: { PackageLocale: locale, ...manifest } as WinGetLocaleSchema,
    };

    return response;
  } catch (error) {
    const message = String(error);
    if (message.includes("Not Found")) {
      return createWinGetError(
        event,
        404,
        `Locale '${locale}' not found for version ${version} of package '${packageId}'`,
      );
    }
    return createWinGetError(event, 500, `Failed to parse locale manifest: ${message}`);
  }
});
