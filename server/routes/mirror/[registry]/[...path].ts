import { defineCachedHandler } from "nitro/cache";
import { getRouterParam } from "nitro/h3";
import { HTTPError, proxyRequest } from "h3";
import { mirrorRegistries } from "../../../utils/mirror";

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
    swr: true,
    group: "mirror",
  },
);
