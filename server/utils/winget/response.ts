import { type H3Event } from "nitro/h3";

import type { WinGetError } from "./types";

/**
 * Create a WinGet-compatible HTTP error response
 */
export function createWinGetError(event: H3Event, status: number, message: string): WinGetError[] {
  event.res.status = status;
  return [{ ErrorCode: status, ErrorMessage: message }];
}
