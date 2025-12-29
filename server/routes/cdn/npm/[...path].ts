import { defineHandler, getRouterParam } from "nitro/h3";
import { HTTPError } from "h3";
import { parseTarGzip } from "nanotar";
import { lookup } from "mrmime";

/**
 * CDN npm route handler
 *
 * Supported path formats:
 * - Scoped:   @scope/package@version/path or @scope/package/path
 * - Normal:    package@version/path or package/path
 */
export default defineHandler(async (event) => {
  const path = getRouterParam(event, "path");
  if (!path) {
    throw new HTTPError({ status: 400, statusText: "Invalid path" });
  }

  // Parse path: @scope/package or package
  let packageName: string;
  let version: string;
  let filepath: string;

  if (path.startsWith("@")) {
    // Scoped: @types/hast@latest/index.d.ts
    const match = path.match(/^@([^/]+)\/([^@/]+)(?:@([^/]+))?(?:\/(.*))?$/);
    if (!match) {
      throw new HTTPError({
        status: 400,
        statusText: "Invalid scoped package path format",
      });
    }

    const [, scope, pkg, pkgVersion, pkgFilepath] = match;
    packageName = `@${scope}/${pkg}`;
    version = pkgVersion || "latest";
    filepath = pkgFilepath || "";
  } else {
    // Normal: uikit@latest/dist/js/uikit.js
    const match = path.match(/^([^@/]+)(?:@([^/]+))?(?:\/(.*))?$/);
    if (!match) {
      throw new HTTPError({
        status: 400,
        statusText: "Invalid package path format",
      });
    }

    const [, pkg, pkgVersion, pkgFilepath] = match;
    if (!pkg) {
      throw new HTTPError({
        status: 400,
        statusText: "Package name is required",
      });
    }
    packageName = pkg;
    version = pkgVersion || "latest";
    filepath = pkgFilepath || "";
  }

  // Fetch package metadata from npm registry
  const registryUrl = "https://registry.npmjs.org";
  const metadataRes = await fetch(`${registryUrl}/${packageName}`);
  if (!metadataRes.ok) {
    throw new HTTPError({
      status: 404,
      statusText: "Package not found",
    });
  }

  const metadata = await metadataRes.json();
  const distTags = metadata["dist-tags"];

  // Try to get specified version, fallback to latest if not found
  let versionInfo = metadata.versions[version];
  if (!versionInfo && distTags?.latest) {
    const latest = distTags.latest;
    if (latest) {
      versionInfo = metadata.versions[latest];
    }
  }

  if (!versionInfo) {
    throw new HTTPError({
      status: 404,
      statusText: "Version not found",
    });
  }

  // Download tarball
  const tarballUrl = versionInfo.dist.tarball;
  const tarballRes = await fetch(tarballUrl);
  if (!tarballRes.ok) {
    throw new HTTPError({
      status: 502,
      statusText: "Failed to download tarball",
    });
  }

  const tarballData = new Uint8Array(await tarballRes.arrayBuffer());

  // Decompress and parse .tar.gz in one step using nanotar
  const files = await parseTarGzip(tarballData);

  // Determine root directory in tarball
  // Scoped: @types/hast -> hast/
  // Normal: uikit -> package/
  const firstFile = files[0];
  const rootDir = firstFile?.name.split("/")[0] || "package";
  const rootPath = `${rootDir}/`;

  // Check if path ends with / (directory listing vs main file)
  const hasTrailingSlash = path.endsWith("/");

  // Special handling for package root access
  if (!filepath) {
    if (hasTrailingSlash) {
      // /cdn/npm/uikit/ -> list directory contents
      const dirContents = files
        .filter((file) => {
          // Exclude the directory entry itself
          if (file.name === rootPath.slice(0, -1)) return false;
          return true;
        })
        .map((file) => ({
          name: file.name.slice(rootPath.length),
          type: file.type,
          size: file.size,
        }))
        .sort((a, b) => {
          const aIsDir = a.type !== "file";
          const bIsDir = b.type !== "file";
          if (aIsDir && !bIsDir) return -1;
          if (!aIsDir && bIsDir) return 1;
          return a.name.localeCompare(b.name);
        });

      event.res.headers.set("Content-Type", "application/json");
      event.res.headers.set("Cache-Control", "public, max-age=600");

      return {
        path: "",
        type: "directory",
        files: dirContents,
      };
    } else {
      // /cdn/npm/uikit -> return package.json main file
      const packageJsonPath = `${rootPath}package.json`;
      const packageJsonFile = files.find(
        (file) => file.name === packageJsonPath,
      );

      if (!packageJsonFile || !packageJsonFile.data) {
        throw new HTTPError({
          status: 404,
          statusText: "package.json not found",
        });
      }

      const packageJson = JSON.parse(
        new TextDecoder().decode(packageJsonFile.data),
      );

      // Determine entry file following jsDelivr priority:
      // jsdelivr > browser > main > module > exports["."] > index.js
      let entryFile =
        packageJson.jsdelivr ||
        packageJson.browser ||
        packageJson.main ||
        packageJson.module ||
        "index.js";

      // Handle exports field (lower priority than the above fields)
      if (
        !packageJson.jsdelivr &&
        !packageJson.browser &&
        !packageJson.main &&
        packageJson.exports
      ) {
        if (typeof packageJson.exports === "string") {
          entryFile = packageJson.exports;
        } else if (packageJson.exports["."]) {
          const exportEntry = packageJson.exports["."];
          entryFile =
            typeof exportEntry === "string"
              ? exportEntry
              : exportEntry.default || entryFile;
        }
      }

      const fullPath = `${rootPath}${entryFile}`;
      const targetFile = files.find((file) => file.name === fullPath);

      if (!targetFile || targetFile.type !== "file" || !targetFile.data) {
        throw new HTTPError({
          status: 404,
          statusText: `Entry file not found: ${entryFile}`,
        });
      }

      const contentType = lookup(entryFile) || "application/octet-stream";

      event.res.headers.set("Content-Type", contentType);
      event.res.headers.set(
        "Cache-Control",
        "public, max-age=31536000, immutable",
      );

      return Buffer.from(targetFile.data);
    }
  }

  // For non-root paths, proceed with normal file/directory lookup
  const fullPath = `${rootPath}${filepath}`;
  const targetFile = files.find((file) => file.name === fullPath);

  // If exact file found, return file content
  if (targetFile && targetFile.type === "file") {
    if (!targetFile.data) {
      throw new HTTPError({
        status: 500,
        statusText: "File data is empty",
      });
    }

    // Determine content type using mrmime
    const contentType = lookup(filepath) || "application/octet-stream";

    // Set response headers for file
    event.res.headers.set("Content-Type", contentType);
    event.res.headers.set(
      "Cache-Control",
      "public, max-age=31536000, immutable",
    );

    return Buffer.from(targetFile.data);
  }

  // If no exact file match, treat as directory and list contents
  const dirPath = `${rootPath}${filepath}/`;

  // Filter and map all files in the directory (recursive)
  const dirContents = files
    .filter((file) => {
      // Include all files that start with the directory path
      if (!file.name.startsWith(dirPath)) return false;

      // Exclude the directory entry itself
      if (file.name === dirPath.slice(0, -1)) return false;

      return true;
    })
    .map((file) => ({
      name: file.name.slice(rootPath.length),
      type: file.type,
      size: file.size,
    }))
    .sort((a, b) => {
      // Sort: directories first (by path), then files (by path)
      const aIsDir = a.type !== "file";
      const bIsDir = b.type !== "file";

      if (aIsDir && !bIsDir) return -1;
      if (!aIsDir && bIsDir) return 1;

      return a.name.localeCompare(b.name);
    });

  // If directory is empty, return 404
  if (dirContents.length === 0) {
    throw new HTTPError({
      status: 404,
      statusText: `Path not found: ${filepath}`,
    });
  }

  // Return directory listing as JSON
  event.res.headers.set("Content-Type", "application/json");
  event.res.headers.set("Cache-Control", "public, max-age=600");

  return {
    path: filepath,
    type: "directory",
    files: dirContents,
  };
});
