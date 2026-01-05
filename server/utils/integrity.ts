import { createHash } from "node:crypto";

/**
 * Calculate SHA-256 hash for Subresource Integrity (SRI)
 *
 * Uses Node.js native crypto API for better type safety and performance.
 *
 * @param data - File data as Uint8Array
 * @returns SRI integrity string in format "sha256-{base64-hash}"
 *
 * @example
 * ```typescript
 * const integrity = await calculateIntegrity(fileData);
 * // Returns: "sha256-x6DyU7IOamUJA7WZXtPnTLMVXbTKfvqHw9hgqF39HXfvhLf3mMNEatcCg+imorU9="
 * ```
 */
export async function calculateIntegrity(data: Uint8Array): Promise<string> {
  // Use Node.js native crypto for SHA-256 (fully type-safe)
  const hash = createHash("sha256");
  hash.update(Buffer.from(data));
  const digest = hash.digest("base64");

  // Return complete SRI format
  return `sha256-${digest}`;
}
