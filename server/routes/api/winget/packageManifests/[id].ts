import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getQuery, getRouterParam } from "nitro/h3";
import type { H3Event } from "nitro/h3";

import {
  buildPackageIndex,
  fetchManifestContent,
  getVersionManifests,
  getDefaultLocaleManifestPath,
  compareVersion,
  createWinGetError,
} from "../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Package Manifests", "Get"],
    summary: "Get package manifest",
    description: "Retrieve a full package manifest with all versions, locales, and installers",
    parameters: [
      {
        in: "path",
        name: "id",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "Version",
        description: "Filter by specific version",
        required: false,
        schema: { type: "string" },
      },
      {
        in: "query",
        name: "Channel",
        description: "Filter by channel",
        required: false,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Package manifest",
      },
      404: {
        description: "Package not found",
      },
    },
  },
});

interface VersionManifest {
  PackageVersion: string;
  DefaultLocale?: string;
  Channel?: string | null;
  Locales?: Record<string, any>[];
  Installers?: Record<string, any>[];
  Manifest?: Record<string, any>;
}

/**
 * GET /packageManifests/{PackageIdentifier}
 *
 * WinGet.RestSource API - Get package manifest
 * Supports query parameters: Version, Channel, Market
 */
export default defineHandler(async (event: H3Event) => {
  const packageId = getRouterParam(event, "id");
  const query = getQuery(event);
  const filterVersion = query.Version as string | undefined;
  const filterChannel = query.Channel as string | undefined;

  if (!packageId) {
    return createWinGetError(event, 400, "PackageIdentifier is required");
  }

  const packageIndex = await buildPackageIndex(event);
  let versions = packageIndex.get(packageId);

  if (!versions || versions.size === 0) {
    return createWinGetError(event, 404, `Package '${packageId}' not found`);
  }

  // Apply version filter if provided
  if (filterVersion) {
    if (versions.has(filterVersion)) {
      versions = new Set([filterVersion]);
    } else {
      return createWinGetError(
        event,
        404,
        `Version ${filterVersion} not found for package '${packageId}'`,
      );
    }
  }

  // Build manifest for all versions (sorted descending)
  const sortedVersions = Array.from(versions).sort((a, b) => compareVersion(b, a));
  const manifestVersions: VersionManifest[] = [];

  await Promise.allSettled(
    sortedVersions.map(async (version) => {
      const manifestFiles = getVersionManifests(packageId, version);
      if (manifestFiles.length === 0) return;

      const versionEntry: VersionManifest = { PackageVersion: version };

      for (const manifestPath of manifestFiles) {
        const filename = manifestPath.split("/").pop()!;
        const content = await fetchManifestContent(manifestPath);
        const parsed = parseYAML(content) as Record<string, any>;

        if (filename === `${packageId}.yaml`) {
          // Version manifest
          versionEntry.DefaultLocale = parsed.DefaultLocale;
          versionEntry.Channel = parsed.Channel;
          versionEntry.Manifest = parsed;

          // Fetch locale manifest using DefaultLocale from version manifest
          if (parsed.DefaultLocale) {
            try {
              const localeContent = await fetchManifestContent(
                getDefaultLocaleManifestPath(packageId, version, parsed.DefaultLocale),
              );
              const localeParsed = parseYAML(localeContent) as Record<string, any>;
              if (!versionEntry.Locales) versionEntry.Locales = [];
              versionEntry.Locales.push(localeParsed);
            } catch {
              // locale file may not exist
            }
          }
        } else if (filename.match(/\.installer\.yaml$/)) {
          // Installer manifest
          versionEntry.Installers = parsed.Installers;
        }
      }

      // Apply channel filter if provided
      if (filterChannel && versionEntry.Channel !== filterChannel) return;

      manifestVersions.push(versionEntry);
    }),
  );

  return {
    Data: {
      PackageIdentifier: packageId,
      Versions: manifestVersions,
    },
  };
});
