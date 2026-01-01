import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseYAML } from "confbox";
import type { VersionSingleResponse } from "../../../../../../utils/winget";
import { getVersionManifests, fetchManifestContent } from "../../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
    summary: "Get specific version of a WinGet package",
    description: "Retrieve detailed manifest information for a specific package version",
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
        description: "Successful response with manifest content",
      },
      404: {
        description: "Package or version not found",
      },
    },
  },
});

/**
 * GET /registry/winget/packages/{PackageIdentifier}/versions/{PackageVersion}
 *
 * WinGet.RestSource API - Get specific version with all manifests
 *
 * Response: VersionSingleResponse with manifest content
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

    const response: VersionSingleResponse = {
      PackageVersion: version,
    };

    // Parse manifest files
    for (const file of manifestFiles) {
      const filename = file.path.split("/").pop()!;

      if (filename === `${packageId}.yaml`) {
        // Main manifest
        try {
          const content = await fetchManifestContent(file.path);
          response.Manifest = parseYAML(content);
        } catch {
          // Skip if parsing fails
        }
      } else if (filename.startsWith(`${packageId}.locale.`)) {
        // Locale manifest
        if (!response.LocaleManifests) {
          response.LocaleManifests = {};
        }

        // Extract locale code: ByteDance.Doubao.locale.en-US.yaml → en-US
        const localeMatch = filename.match(/\.locale\.([^.]+)\.yaml$/);
        if (localeMatch && localeMatch[1]) {
          const locale = localeMatch[1];
          try {
            const content = await fetchManifestContent(file.path);
            response.LocaleManifests[locale] = parseYAML(content);
          } catch {
            // Skip if parsing fails
          }
        }
      } else if (filename === `${packageId}.installer.yaml`) {
        // Installer manifest
        try {
          const content = await fetchManifestContent(file.path);
          response.InstallerManifest = parseYAML(content);
        } catch {
          // Skip if parsing fails
        }
      }
    }

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 3600,
    swr: true,
    group: "registry:winget",
  },
);
