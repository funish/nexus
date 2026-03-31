import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, getRouterParam } from "nitro/h3";

import {
  getVersionManifests,
  fetchManifestContent,
} from "../../../../../../../utils/winget/manifest";
import { createWinGetError } from "../../../../../../../utils/winget/response";
import type { LocaleMultipleResponse, LocaleSchema } from "../../../../../../../utils/winget/types";

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
        in: "query",
        name: "ContinuationToken",
        description: "Pagination token",
        required: false,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Locale list",
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
 * GET /packages/{PackageIdentifier}/versions/{PackageVersion}/locales
 *
 * WinGet.RestSource API - Get all locales for a version
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

  // Separate main manifest from additional locale files
  const mainManifestFile = manifestFiles.find(
    (path) => path.split("/").pop() === `${packageId}.yaml`,
  );
  const localeFiles = manifestFiles.filter((path) => path.includes(".locale."));

  // Fetch main manifest once to determine if it provides default locale data
  let mainLocaleEntry: LocaleSchema | null = null;
  if (mainManifestFile) {
    try {
      const content = await fetchManifestContent(mainManifestFile);
      const manifest = parseYAML(content) as Record<string, any>;
      const defaultLocale = manifest.DefaultLocale || manifest.PackageLocale;
      const hasLocaleData = Boolean(
        manifest.PackageLocale || manifest.Publisher || manifest.PackageName,
      );
      const hasDefaultLocaleFile = defaultLocale
        ? localeFiles.some((path) => path.includes(`.locale.${defaultLocale}.yaml`))
        : false;

      if (hasLocaleData && !hasDefaultLocaleFile) {
        mainLocaleEntry = {
          PackageLocale: defaultLocale || manifest.PackageLocale,
          ...manifest,
        } as LocaleSchema;
      }
    } catch {
      // Ignore
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

  const PAGE_SIZE = 25;

  // If main locale is included and falls within the current page, account for it
  const localeStartIndex = mainLocaleEntry && startIndex === 0 ? 0 : Math.max(0, startIndex - 1);
  const paginatedLocaleFiles = localeFiles.slice(
    localeStartIndex,
    localeStartIndex + PAGE_SIZE - (mainLocaleEntry && startIndex === 0 ? 1 : 0),
  );

  const results = await Promise.allSettled(
    paginatedLocaleFiles.map(async (localePath) => {
      const filename = localePath.split("/").pop()!;
      const localeMatch = filename.match(/\.locale\.([^.]+)\.yaml$/);
      if (!localeMatch) return null;

      const content = await fetchManifestContent(localePath);
      const manifest = parseYAML(content) as Record<string, any>;

      return {
        PackageLocale: localeMatch[1],
        ...manifest,
      } as LocaleSchema;
    }),
  );

  const locales: LocaleSchema[] = [];

  // Include main locale on the first page
  if (mainLocaleEntry && startIndex === 0) {
    locales.push(mainLocaleEntry);
  }

  for (const result of results) {
    if (result.status === "fulfilled" && result.value) {
      locales.push(result.value);
    }
  }

  const response: LocaleMultipleResponse = {
    Data: locales,
  };

  const totalLocales = (mainLocaleEntry ? 1 : 0) + localeFiles.length;
  if (startIndex + PAGE_SIZE < totalLocales) {
    response.ContinuationToken = Buffer.from((startIndex + PAGE_SIZE).toString()).toString(
      "base64",
    );
  }

  return response;
});
