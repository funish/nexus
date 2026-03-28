import { defineRouteMeta } from "nitro";
import { defineHandler } from "nitro/h3";

defineRouteMeta({
  openAPI: {
    tags: ["WinGet API"],
    summary: "Get server information",
    description: "Returns server information including supported API versions and capabilities",
    responses: {
      200: {
        description: "Server information",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                SourceIdentifier: { type: "string" },
                ServerSupportedVersions: {
                  type: "array",
                  items: { type: "string" },
                },
                RequiredPackageMatchFields: {
                  type: "array",
                  items: { type: "string" },
                },
                UnsupportedPackageMatchFields: {
                  type: "array",
                  items: { type: "string" },
                },
              },
              required: ["SourceIdentifier", "ServerSupportedVersions"],
            },
          },
        },
      },
    },
  },
});

/**
 * GET /information
 *
 * WinGet.RestSource API - Server information
 * This endpoint is required for winget client source discovery.
 */
export default defineHandler(() => {
  return {
    Data: {
      SourceIdentifier: "Funish.Nexus",
      ServerSupportedVersions: ["1.4.0", "1.9.0"],
      RequiredPackageMatchFields: ["PackageIdentifier"],
      UnsupportedPackageMatchFields: ["Market", "HasInstallerType"],
    },
  };
});
