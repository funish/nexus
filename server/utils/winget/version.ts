/**
 * Compare two version strings using Bun.semver.order, falling back to
 * numeric dot-separated comparison for non-semver versions (e.g. "10.0.4.1").
 */
export function compareVersion(a: string, b: string): number {
  try {
    return Bun.semver.order(a, b);
  } catch {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    const len = Math.max(pa.length, pb.length);
    for (let i = 0; i < len; i++) {
      const x = pa[i] || 0;
      const y = pb[i] || 0;
      if (x !== y) return x - y;
    }
    return 0;
  }
}
