/**
 * ContinuationToken encode/decode utilities.
 *
 * WinGet REST Source spec uses base64-encoded offsets as pagination tokens.
 */

/**
 * Decode a continuation token into a numeric offset.
 * Returns 0 on missing or malformed input.
 */
export function decodeContinuationToken(token: string | undefined): number {
  if (!token) return 0;
  try {
    return parseInt(Buffer.from(token, "base64").toString(), 10);
  } catch {
    return 0;
  }
}

/**
 * Encode a numeric offset into a continuation token.
 */
export function encodeContinuationToken(offset: number): string {
  return Buffer.from(offset.toString()).toString("base64");
}
