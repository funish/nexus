import { semver } from "bun";
import { parseYAML } from "confbox";
import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";

import type { VersionMultipleResponse, VersionSchema } from "../../../../../utils/winget";
import {
  buildPackageIndex,
  createWinGetError,
  fetchManifestContent,
  getVersionManifests,
} from "../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["Versions", "Get"],
    summary: "Get all versions of a WinGet package",
    description: "Retrieve all available versions for a specific WinGet package",
    parameters: [
      {
        in: "path",
        name: "id",
        description: "Package identifier",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Successful response",
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
                      PackageVersion: { type: "string" },
                      DefaultLocale: { type: "string" },
                      Channel: { type: "string" },
                    },
                    required: ["PackageVersion", "DefaultLocale"],
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
        description: "Package not found",
      },
    },
  },
});

export default defineHandler(async (event) => {
  const packageId = getRouterParam(event, "id");

  if (!packageId) {
    return createWinGetError(event, 400, "PackageIdentifier is required");
  }

  // Build package index
  const packageIndex = await buildPackageIndex(event);
  const versions = packageIndex.get(packageId);

  if (!versions) {
    return createWinGetError(event, 404, `Package '${packageId}' not found`);
  }

  const sortedVersions = Array.from(versions).sort((a, b) => semver.order(b, a));

  // For each version, fetch the version manifest to get DefaultLocale and Channel
  const manifestPromises = sortedVersions.map(async (version) => {
    const manifestFiles = getVersionManifests(packageId, version);
    const versionManifestPath = manifestFiles[0]; // {id}.yaml

    if (!versionManifestPath) {
      return {
        version,
        defaultLocale: "en-US",
        channel: undefined,
      };
    }

    try {
      const content = await fetchManifestContent(versionManifestPath);
      const manifest = parseYAML(content) as Record<string, any>;
      return {
        version,
        defaultLocale: (manifest.DefaultLocale as string) || "en-US",
        channel: manifest.Channel as string | undefined,
      };
    } catch {
      return {
        version,
        defaultLocale: "en-US",
        channel: undefined,
      };
    }
  });

  // Wait for all manifest fetches to complete
  const manifestResults = await Promise.allSettled(manifestPromises);

  // Build version data array
  const versionData: VersionSchema[] = manifestResults.map((result, index) => {
    const version = sortedVersions[index];

    if (!version) {
      return {
        PackageVersion: "unknown",
        DefaultLocale: "en-US",
      };
    }

    const data =
      result.status === "fulfilled"
        ? result.value
        : {
            version,
            defaultLocale: "en-US",
            channel: undefined,
          };

    return {
      PackageVersion: data.version,
      DefaultLocale: data.defaultLocale,
      ...(data.channel && { Channel: data.channel }),
    };
  });

  const response: VersionMultipleResponse = {
    Data: versionData,
  };

  event.res.headers.set("Content-Type", "application/json");

  return response;
});
