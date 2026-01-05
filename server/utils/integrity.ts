import { subtle } from "uncrypto";
import { toArrayBuffer, toBase64 } from "undio";

/**
 * Calculate SHA-256 hash for Subresource Integrity (SRI)
 *
 * Uses uncrypto's unified Web Crypto API with undio for type-safe conversions.
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
  // Convert Uint8Array to ArrayBuffer (type-safe)
  const arrayBuffer = await toArrayBuffer(data);

  // Use uncrypto's unified Web Crypto API for SHA-256
  const hashBuffer = await subtle.digest({ name: "SHA-256" }, arrayBuffer);

  // Convert hash buffer to Base64 (type-safe)
  const base64 = await toBase64(hashBuffer);

  // Return complete SRI format
  return `sha256-${base64}`;
}
