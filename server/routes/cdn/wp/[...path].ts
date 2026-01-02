import { defineRouteMeta } from "nitro";
import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { getContentType } from "../../../utils/mime";
import { useStorage } from "nitro/storage";

defineRouteMeta({
  openAPI: {
    tags: ["CDN"],
    summary: "WordPress plugins and themes CDN",
    description: "Access WordPress plugin and theme files from WordPress SVN repository",
    parameters: [
      {
        in: "path",
        name: "path",
        description:
          "WordPress resource path (plugins/name/tags/version/file or themes/name/version/file)",
        required: true,
        schema: { type: "string" },
      },
    ],
    responses: {
      200: {
        description: "Returns file content with appropriate content-type",
      },
      404: {
        description: "Resource not found",
      },
    },
  },
});

/**
 * Get appropriate cache-control header based on version type
 */
function getWpCacheControl(isTrunk: boolean): string {
  // Trunk changes frequently - short cache (10 minutes)
  if (isTrunk) {
    return "public, max-age=600";
  }
  // Tags are immutable - long cache (1 year, immutable)
  return "public, max-age=31536000, immutable";
}

/**
 * Fetch and cache a single file from WordPress SVN with jsDelivr fallback
 * Returns immediately without blocking on cache write
 */
async function fetchAndCacheFile(
  svnUrl: string,
  cacheKey: string,
  jsDelivrUrl?: string,
): Promise<Uint8Array> {
  const storage = useStorage("cache");

  // Check cache first
  const cached = await storage.getItemRaw(cacheKey);
  if (cached) {
    return new Uint8Array(cached);
  }

  // Try WordPress SVN first
  const response = await fetch(svnUrl);
  if (response.ok) {
    const data = await response.bytes();

    // Cache in background without blocking response
    storage.setItemRaw(cacheKey, data).catch((err) => {
      console.error(`Failed to cache WordPress file:`, err);
    });

    return data;
  }

  // If WordPress SVN fails and jsDelivr URL is provided, try jsDelivr
  if (jsDelivrUrl) {
    const jsDelivrResponse = await fetch(jsDelivrUrl);
    if (jsDelivrResponse.ok) {
      const data = await jsDelivrResponse.bytes();

      // Cache in background without blocking response
      storage.setItemRaw(cacheKey, data).catch((err) => {
        console.error(`Failed to cache WordPress file:`, err);
      });

      return data;
    }
  }

  // Both failed - throw error
  throw new HTTPError({
    status: response.status === 404 ? 404 : 502,
    statusText:
      response.statusText || `Failed to fetch from WordPress SVN (HTTP ${response.status})`,
  });
}

/**
 * CDN WordPress route handler
 *
 * Supported path formats:
 * - Plugins: /wp/plugins/plugin-name/tags/version/file or /wp/plugins/plugin-name/trunk/file
 * - Themes:  /wp/themes/theme-name/version/file
 *
 * Examples:
 * - /cdn/wp/plugins/wp-slimstat/tags/4.6.5/wp-slimstat.js
 * - /cdn/wp/plugins/wp-slimstat/trunk/wp-slimstat.js
 * - /cdn/wp/themes/twenty-eighteen/1.7/assets/js/html5.js
 */
export default defineHandler(async (event) => {
  const path = getRouterParam(event, "path");

  if (!path) {
    throw new HTTPError({ status: 400, statusText: "Invalid path" });
  }

  let svnUrl: string;
  let jsDelivrUrl: string | undefined;
  let cacheKey: string;
  let isTrunk = false;

  // Check if it's a theme
  if (path.startsWith("themes/")) {
    // /wp/themes/theme-name/version/file
    const themeMatch = path.match(/^themes\/([^/]+)\/([^/]+)\/(.*)$/);
    if (!themeMatch) {
      throw new HTTPError({
        status: 400,
        statusText: "Invalid WordPress theme path format",
      });
    }

    const [, themeName, version, filepath] = themeMatch;
    svnUrl = `https://themes.svn.wordpress.org/${themeName}/${version}/${filepath}`;
    jsDelivrUrl = `https://cdn.jsdelivr.net/wp/themes/${themeName}/${version}/${filepath}`;
    cacheKey = `cdn/wp/themes/${themeName}/${version}/${filepath}`;
  } else {
    // /wp/plugins/plugin-name/tags/version/file or /wp/plugins/plugin-name/trunk/file
    const pluginMatch = path.match(/^plugins\/([^/]+)\/(tags|trunk)(?:\/([^/]+))?(?:\/(.*))?$/);
    if (!pluginMatch) {
      throw new HTTPError({
        status: 400,
        statusText: "Invalid WordPress plugin path format",
      });
    }

    const [, pluginName, ref, version, filepath] = pluginMatch;
    isTrunk = ref === "trunk";

    // For tags: /wp/plugins/plugin-name/tags/1.0/file
    // For trunk: /wp/plugins/plugin-name/trunk/file (no version)
    if (ref === "trunk") {
      svnUrl = `https://plugins.svn.wordpress.org/${pluginName}/trunk/${filepath || ""}`;
      jsDelivrUrl = `https://cdn.jsdelivr.net/wp/${pluginName}/trunk/${filepath || ""}`;
      cacheKey = `cdn/wp/plugins/${pluginName}/trunk/${filepath || ""}`;
    } else {
      if (!version || !filepath) {
        throw new HTTPError({
          status: 400,
          statusText:
            "Invalid WordPress plugin path format. Use /wp/plugins/plugin-name/tags/version/file",
        });
      }
      svnUrl = `https://plugins.svn.wordpress.org/${pluginName}/tags/${version}/${filepath}`;
      jsDelivrUrl = `https://cdn.jsdelivr.net/wp/${pluginName}/tags/${version}/${filepath}`;
      cacheKey = `cdn/wp/plugins/${pluginName}/tags/${version}/${filepath}`;
    }
  }

  // Fetch and cache file with jsDelivr fallback
  const fileData = await fetchAndCacheFile(svnUrl, cacheKey, jsDelivrUrl);

  // Get content type
  const filepath = svnUrl.split("/").pop() || "";
  const contentType = getContentType(filepath);

  // Set headers
  event.res.headers.set("Content-Type", contentType);
  event.res.headers.set("Cache-Control", getWpCacheControl(isTrunk));

  // Return file content
  return Buffer.from(fileData);
});
