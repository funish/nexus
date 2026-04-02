import semver from "semver";

import { cacheStorage } from "../storage";

export interface BundleOptions {
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
  const storage = cacheStorage;
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
      const minVersion = semver.minVersion(depRange as string);
      if (minVersion) {
        dependencies[depName] = minVersion.version;
      }
    } catch {
      console.error(`[Bundler] Error resolving ${depName}`);
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

  // Normalize entry point to match normalized file keys
  const normalizedEntryPoint = entryPoint.replace(/^\.?\//, "");

  // Bundle with Bun.build
  const buildResult = await Bun.build({
    entrypoints: [`/virtual/${packageName}/${normalizedEntryPoint}`],
    root: "/",
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
            if (args.path in files) {
              return { path: args.path, namespace: "virtual" };
            }
            return null;
          });

          // Handle relative imports from virtual files
          build.onResolve({ filter: /^\.\.?\// }, (args) => {
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
  const allImports = new Set<string>();

  // Find all bare imports in the code
  const importRegex = /(?:import|export)\s*(?:\*|\{[^}]*\}|\w+)?\s*from\s*["']([^"']+)["']/g;
  let match;
  while ((match = importRegex.exec(bundledCode)) !== null) {
    const importPath = match[1];
    if (importPath && !importPath.startsWith(".") && !importPath.startsWith("/")) {
      allImports.add(importPath);
    }
  }

  // Rewrite all bare imports to CDN paths
  for (const depName of allImports) {
    const cdnPath = cdnPaths[depName] || `/cdn/npm/${depName}/+esm`;
    const escapedDepName = depName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    bundledCode = bundledCode.replaceAll(
      new RegExp(`(from\\s*["'])${escapedDepName}(["'])`, "g"),
      `$1${cdnPath}$2`,
    );
  }

  return bundledCode;
}
