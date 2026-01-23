import { useStorage } from "nitro/storage";
import semver from "semver";

interface BundleOptions {
  packageName: string;
  version: string;
  entryPoint: string;
}

/**
 * Bundle an npm package using Bun.build from cached files
 * Returns a bundled ESM module as a string
 */
export async function bundleNpmPackage(options: BundleOptions): Promise<string> {
  const { packageName, version, entryPoint } = options;
  const storage = useStorage("cache");
  const cacheBase = `cdn/npm/${packageName}/${version}`;

  // Check if package is cached
  const meta = await storage.getMeta(cacheBase);
  if (!meta?.files) {
    throw new Error(`Package ${packageName}@${version} is not cached yet`);
  }

  // Read package.json to get dependency ranges
  const packageJsonKey = `${cacheBase}/package.json`;
  const packageJsonData = await storage.getItemRaw(packageJsonKey);

  if (!packageJsonData) {
    throw new Error(`package.json not found for ${packageName}@${version}`);
  }

  const packageJson = JSON.parse(new TextDecoder().decode(packageJsonData));
  const dependencyRanges = packageJson.dependencies || {};
  const peerDependencies = packageJson.peerDependencies || {};

  // Merge all dependency ranges
  const allDependencyRanges = { ...dependencyRanges, ...peerDependencies };

  // Resolve dependency ranges to exact versions using semver
  const dependencies: Record<string, string> = {};

  for (const [depName, depRange] of Object.entries(allDependencyRanges)) {
    try {
      const rangeStr = depRange as string;
      // Use semver to parse the range and get the upper bound
      const range = new semver.Range(rangeStr);

      // Get comparators from the range to find the upper bound
      // For "1 - 2", this will be >=1.0.0 <3.0.0-0, so we want 2
      // For "^1.2.3", this will be >=1.2.3 <2.0.0-0, so we want 1
      // For "~1.2.3", this will be >=1.2.3 <1.3.0-0, so we want 1.2
      let targetVersion: string | null = null;

      for (const comparatorSet of range.set) {
        for (const comparator of comparatorSet) {
          // Look for the upper bound (comparator with < operator)
          if (comparator.operator === "<") {
            const version = comparator.semver;
            // For <3.0.0-0, we want 2 (major - 1)
            // For <2.0.0-0, we want 1 (major - 1)
            // For <1.3.0-0, we want 1.2 (major.minor-1)
            if (version.patch === 0 && version.prerelease && version.prerelease[0] === 0) {
              // This is the upper bound format like <3.0.0-0
              const major = version.major;
              const minor = version.minor;

              if (minor === 0) {
                // <3.0.0-0 -> use 2
                targetVersion = String(major - 1);
              } else {
                // <1.3.0-0 -> use 1.2
                targetVersion = `${major}.${minor - 1}`;
              }
              break;
            }
          }
        }
        if (targetVersion) break;
      }

      // Fallback: if no upper bound found, use minVersion
      if (!targetVersion) {
        const minVersion = semver.minVersion(rangeStr);
        if (minVersion) {
          targetVersion = minVersion.version;
        }
      }

      if (targetVersion) {
        dependencies[depName] = targetVersion;
      }
    } catch (error) {
      // If semver parsing fails, skip this dependency
      console.error(`[Bundler] Error resolving ${depName}:`, error);
    }
  }

  // Load all files into memory for Bun.build
  const fileList = meta.files as Array<{ name: string; size: number }>;
  const files: Record<string, string> = {};

  for (const file of fileList) {
    const cacheKey = `${cacheBase}/${file.name}`;
    const data = await storage.getItemRaw(cacheKey);
    if (data) {
      // Normalize filename: remove leading ./ or /
      const normalizedName = file.name.replace(/^\.?\//, "");
      const key = `/virtual/${packageName}/${normalizedName}`;
      try {
        const content = new TextDecoder().decode(data);
        files[key] = content;
      } catch {
        // Skip files that can't be decoded (likely binary)
      }
    }
  }

  // Build external dependencies list and CDN paths mapping
  const external: string[] = [];
  const cdnPaths: Record<string, string> = {};
  for (const [depName, depVersion] of Object.entries(dependencies)) {
    external.push(depName);
    cdnPaths[depName] = `/cdn/npm/${depName}@${depVersion}/+esm`;
  }

  // Bundle with Bun.build
  const buildResult = await Bun.build({
    entrypoints: [`/virtual/${packageName}/${entryPoint}`],
    root: "/", // Set root to avoid file system resolution issues
    target: "browser",
    format: "esm",
    external: external,
    minify: true,
    sourcemap: false,
    plugins: [
      {
        name: "memory-resolver",
        setup(build) {
          // Handle module resolution - must filter for virtual paths
          build.onResolve({ filter: /^\/virtual\// }, (args) => {
            // Handle absolute virtual paths
            if (args.path in files) {
              return { path: args.path, namespace: "virtual" };
            }
            return null;
          });

          // Handle relative imports from virtual files
          build.onResolve({ filter: /^\.\.?\// }, (args) => {
            // Only process if the importer is a virtual file
            if (args.importer.startsWith("/virtual/")) {
              const importerDir = args.importer.substring(0, args.importer.lastIndexOf("/"));

              // Resolve relative path
              const parts = importerDir.split("/");
              const sourceParts = args.path.split("/");

              for (const part of sourceParts) {
                if (part === "..") {
                  parts.pop();
                } else if (part !== ".") {
                  parts.push(part);
                }
              }

              const resolved = parts.join("/");
              if (resolved in files) {
                return { path: resolved, namespace: "virtual" };
              }
            }
            return null;
          });

          // Handle bare imports (external dependencies)
          build.onResolve({ filter: /^[^./]/ }, (args) => {
            // Mark as external for bare imports
            return { path: args.path, external: true };
          });

          // Handle module loading
          build.onLoad({ filter: /^\/virtual\//, namespace: "virtual" }, (args) => {
            if (args.path in files) {
              return { contents: files[args.path] as string, loader: "js" };
            }
            return undefined;
          });
        },
      },
    ],
  });

  // Get the bundled code
  const output = buildResult.outputs[0];
  if (!output) {
    throw new Error("Bundler failed to generate output");
  }
  let bundledCode = await output.text();

  // Rewrite external imports to CDN paths
  // Process all dependencies, not just those in the initial list
  // to catch transitive dependencies
  const allImports = new Set<string>();

  // First pass: find all bare imports in the code
  // Matches both: import/export ... from "x" and export*from"x" (minified)
  const importRegex = /(?:import|export)\s*(?:\*|\{[^}]*\}|\w+)?\s*from\s*["']([^"']+)["']/g;
  let match;
  while ((match = importRegex.exec(bundledCode)) !== null) {
    const importPath = match[1];
    if (importPath && !importPath.startsWith(".") && !importPath.startsWith("/")) {
      allImports.add(importPath);
    }
  }

  // Second pass: rewrite all bare imports to CDN paths
  for (const depName of allImports) {
    // Use versioned path if available, otherwise fallback to unversioned path
    const cdnPath = cdnPaths[depName] || `/cdn/npm/${depName}/+esm`;
    // Match both: "dep" and from"dep" (minified), then replace with CDN path
    bundledCode = bundledCode.replaceAll(
      new RegExp(`(?:from)?(["'])${depName}\\1`, "g"),
      (match, quote) =>
        match.startsWith("from") ? `from${quote}${cdnPath}${quote}` : `${quote}${cdnPath}${quote}`,
    );
  }

  return bundledCode;
}
