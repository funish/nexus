import { defineRouteMeta } from "nitro";
import { defineHandler } from "nitro/h3";

defineRouteMeta({
  openAPI: {
    tags: ["Server", "Get"],
    summary: "Get Server Information.",
    parameters: [
      {
        in: "header",
        name: "Windows-Package-Manager",
        description: "Windows Package Manager client version",
        required: false,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Server information",
        content: {
          "application/json": {
            schema: {
              type: "object",
              properties: {
                Data: {
                  type: "object",
                  properties: {
                    SourceIdentifier: { type: "string" },
                    ServerSupportedVersions: { type: "array", items: { type: "string" } },
                    SourceAgreements: {
                      type: "object",
                      properties: {
                        AgreementsIdentifier: { type: "string" },
                        Agreements: {
                          type: "array",
                          items: {
                            type: "object",
                            properties: {
                              AgreementLabel: { type: "string" },
                              Agreement: { type: "string" },
                              AgreementUrl: { type: "string" },
                            },
                          },
                        },
                      },
                    },
                    UnsupportedPackageMatchFields: { type: "array", items: { type: "string" } },
                    RequiredPackageMatchFields: { type: "array", items: { type: "string" } },
                    UnsupportedQueryParameters: { type: "array", items: { type: "string" } },
                    RequiredQueryParameters: { type: "array", items: { type: "string" } },
                    Authentication: {
                      type: "object",
                      properties: {
                        AuthenticationType: { type: "string" },
                      },
                    },
                  },
                  required: ["SourceIdentifier", "ServerSupportedVersions"],
                },
              },
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
 * GET /information
 *
 * WinGet.RestSource API - Server information
 */
export default defineHandler(() => {
  return {
    Data: {
      SourceIdentifier: "Funish.Nexus",
      ServerSupportedVersions: ["1.4.0", "1.9.0"],
      RequiredPackageMatchFields: ["PackageIdentifier"],
      UnsupportedPackageMatchFields: ["Market", "HasInstallerType"],
      UnsupportedQueryParameters: ["FetchAllManifests"],
      RequiredQueryParameters: [],
      Authentication: { AuthenticationType: "none" },
    },
  };
});
