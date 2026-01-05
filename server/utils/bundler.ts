import { rolldown } from "rolldown";
import { useStorage } from "nitro/storage";
import semver from "semver";

interface BundleOptions {
  packageName: string;
  version: string;
  entryPoint: string;
}

/**
 * Bundle an npm package using rolldown from cached files
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

  // Resolve dependency ranges to exact versions using semver
  const dependencies: Record<string, string> = {};

  for (const [depName, depRange] of Object.entries(dependencyRanges)) {
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

  // Load all files into memory
  const fileList = meta.files as Array<{ name: string; size: number }>;
  const memoryFiles = new Map<string, string>();

  for (const file of fileList) {
    const cacheKey = `${cacheBase}/${file.name}`;
    const data = await storage.getItemRaw(cacheKey);
    if (data) {
      const key = `/virtual/${packageName}/${file.name}`;
      try {
        const content = new TextDecoder().decode(data);
        memoryFiles.set(key, content);
      } catch {
        // Skip files that can't be decoded (likely binary)
      }
    }
  }

  // Build paths map for external dependencies
  const paths: Record<string, string> = {};
  for (const [depName, depVersion] of Object.entries(dependencies)) {
    paths[depName] = `/cdn/npm/${depName}@${depVersion}/+esm`;
  }

  // Bundle with rolldown
  const bundle = await rolldown({
    input: `/virtual/${packageName}/${entryPoint}`,
    plugins: [
      {
        name: "memory-resolver",
        resolveId(source, importer) {
          // Handle absolute virtual paths
          if (source.startsWith("/virtual/")) {
            if (memoryFiles.has(source)) {
              return { id: source };
            }
            return null;
          }

          // Handle relative imports
          if (source.startsWith("./") || source.startsWith("../")) {
            if (!importer) return null;

            const lastSlash = importer.lastIndexOf("/");
            const importerDir = lastSlash !== -1 ? importer.substring(0, lastSlash) : importer;

            // Resolve relative path
            const parts = importerDir.split("/");
            const sourceParts = source.split("/");

            for (const part of sourceParts) {
              if (part === "..") {
                parts.pop();
              } else if (part !== ".") {
                parts.push(part);
              }
            }

            const resolved = parts.join("/");
            if (memoryFiles.has(resolved)) {
              return { id: resolved };
            }
            return null;
          }

          // Handle bare imports (external dependencies)
          if (!source.startsWith(".") && !source.startsWith("/")) {
            // Check if it's in dependencies
            if (dependencies[source]) {
              return { id: source, external: true };
            }
            // Unknown dependency, keep as external
            return { id: source, external: true };
          }

          return null;
        },
        load(id) {
          if (memoryFiles.has(id)) {
            return memoryFiles.get(id)!;
          }
          return null;
        },
      },
    ],
  });

  const output = await bundle.generate({
    format: "esm",
    sourcemap: false,
    paths: paths, // Map external dependencies to CDN paths
    minify: true,
  });

  await bundle.close();

  const bundledCode = output.output[0].code;
  return bundledCode;
}
