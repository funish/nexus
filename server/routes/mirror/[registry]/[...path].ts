import { defineRouteMeta } from "nitro";
import { defineCachedHandler } from "nitro/cache";
import { getRouterParam } from "nitro/h3";
import { HTTPError, proxyRequest } from "h3";
import { mirrorRegistries } from "../../../utils/mirror";

defineRouteMeta({
  openAPI: {
    tags: ["Registry Mirror"],
    summary: "Universal registry mirror proxy",
    description:
      "Mirror and proxy requests to various package registries. Supports npm, PyPI, crates.io, and many other package managers.",
    parameters: [
      {
        in: "path",
        name: "registry",
        description: "Registry name (e.g., npm, pypi, crates, go, maven, composer, docker, etc.)",
        required: true,
        schema: {
          type: "string",
          enum: [
            "npm",
            "jsr",
            "pypi",
            "crates",
            "go",
            "maven",
            "gradle",
            "composer",
            "docker",
            "ghcr",
            "quay",
            "pub",
            "hackage",
            "julia",
            "cran",
            "luarocks",
            "nimble",
            "hex",
            "clojars",
            "conan",
            "homebrew",
            "chocolatey",
            "debian",
            "ubuntu",
            "fedora",
            "epel",
            "arch",
            "alpine",
            "gentoo",
            "openwrt",
            "anaconda",
            "condaforge",
            "cpan",
            "ctan",
            "postgresql",
            "mysql",
            "nix",
            "guix",
          ],
        },
      },
      {
        in: "path",
        name: "path",
        description: "Path to proxy to the target registry",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Proxied response from target registry",
      },
      400: {
        description: "Invalid registry name",
      },
      404: {
        description: "Unknown registry or path not found",
      },
      502: {
        description: "Failed to connect to target registry",
      },
    },
  },
});

/**
 * Generic mirror handler for all registries
 *
 * Route pattern: /mirror/:registry/*
 *
 * Examples:
 * - /mirror/npm/jquery
 * - /mirror/jsr/@jsr/std__path
 */
export default defineCachedHandler(
  async (event) => {
    const registry = getRouterParam(event, "registry");
    const path = getRouterParam(event, "path");

    if (!registry) {
      throw new HTTPError({
        status: 400,
        statusText: "Registry name is required",
      });
    }

    const targetUrl = mirrorRegistries[registry];
    if (!targetUrl) {
      throw new HTTPError({
        status: 404,
        statusText: `Unknown registry: ${registry}`,
      });
    }

    return await proxyRequest(event, `${targetUrl}/${path}`);
  },
  {
    maxAge: 600,
  },
);
