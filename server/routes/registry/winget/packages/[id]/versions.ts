import { defineCachedHandler } from "nitro/cache";
import { defineRouteMeta } from "nitro";
import { getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseYAML } from "confbox";
import type { VersionMultipleResponse, VersionSchema } from "../../../../../utils/winget";
import {
  buildPackageIndex,
  fetchManifestContent,
  getLetterDirectoryShas,
  getGitHubTreePaths,
} from "../../../../../utils/winget";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet Registry"],
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

export default defineCachedHandler(
  async (event) => {
    const packageId = getRouterParam(event, "id");

    if (!packageId) {
      throw new HTTPError({
        status: 400,
        statusText: "PackageIdentifier is required",
      });
    }

    // Build package index
    const packageIndex = await buildPackageIndex();
    const versions = packageIndex.get(packageId);

    if (!versions) {
      throw new HTTPError({
        status: 404,
        statusText: `Package '${packageId}' not found`,
      });
    }

    // Get all manifest paths for all versions at once
    // Build version -> manifest path mapping
    const versionManifestMap = new Map<string, string>();

    try {
      const parts = packageId.split(".");
      if (parts.length >= 2) {
        const [publisher, name] = parts;

        if (publisher && name) {
          const letter = publisher[0]?.toLowerCase();

          if (letter) {
            // Get all paths for this package's letter directory
            const letterShas = await getLetterDirectoryShas();
            const sha = letterShas.get(letter);

            if (sha) {
              const paths = await getGitHubTreePaths(sha, `manifests/${letter}`);
              const pathPrefix = `${publisher}/${name}/`;

              // Find all version manifests
              for (const path of paths) {
                if (path.startsWith(pathPrefix) && path.endsWith(`/${packageId}.yaml`)) {
                  // Extract version from path: publisher/name/version/package.yaml
                  const pathParts = path.split("/");
                  const nameIndex = pathParts.indexOf(name);
                  const versionIndex = nameIndex !== -1 ? nameIndex + 1 : -1;

                  if (versionIndex > 0 && versionIndex < pathParts.length) {
                    const version = pathParts[versionIndex];
                    if (version) {
                      versionManifestMap.set(version, `manifests/${letter}/${path}`);
                    }
                  }
                }
              }
            }
          }
        }
      }
    } catch {
      // If building manifest map fails, continue without DefaultLocale
    }

    // Build response with DefaultLocale for each version
    const versionData: VersionSchema[] = [];
    for (const version of Array.from(versions).sort().reverse()) {
      let defaultLocale = "en-US"; // Fallback default
      let channel: string | undefined = undefined;

      // Try to get DefaultLocale from pre-built manifest map
      const manifestPath = versionManifestMap.get(version);
      if (manifestPath) {
        try {
          const content = await fetchManifestContent(manifestPath);
          const manifest = parseYAML(content) as Record<string, any>;
          defaultLocale = (manifest.DefaultLocale as string) || "en-US";
          channel = manifest.Channel as string | undefined;
        } catch {
          // Keep fallback on error
        }
      }

      versionData.push({
        PackageVersion: version,
        DefaultLocale: defaultLocale,
        ...(channel && { Channel: channel }),
      });
    }

    const response: VersionMultipleResponse = {
      Data: versionData,
    };

    event.res.headers.set("Content-Type", "application/json");

    return response;
  },
  {
    maxAge: 600,
    group: "registry:winget",
  },
);
