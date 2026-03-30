import { type H3Event } from "nitro/h3";

import type { WinGetError } from "./types";

/**
 * Create a WinGet-compatible HTTP error response
 */
export function createWinGetError(event: H3Event, status: number, message: string): WinGetError[] {
  event.res.status = status;
  return [{ ErrorCode: status, ErrorMessage: message }];
}

/**
 * Compare two version strings, falling back to numeric comparison for non-semver versions.
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
