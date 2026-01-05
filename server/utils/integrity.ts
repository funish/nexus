import { subtle } from "uncrypto";

/**
 * Calculate SHA-256 hash for Subresource Integrity (SRI)
 *
 * Uses uncrypto's unified Web Crypto API with manual buffer handling
 * to ensure compatibility across all environments including edge workers.
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
  // Create a copy of the data to avoid shared state issues in edge workers
  const uint8Array = new Uint8Array(data);

  // Convert to ArrayBuffer (slice ensures we only get the relevant portion)
  const arrayBuffer = uint8Array.buffer.slice(
    uint8Array.byteOffset,
    uint8Array.byteOffset + uint8Array.byteLength,
  );

  // Use uncrypto's unified Web Crypto API for SHA-256
  const hashBuffer = await subtle.digest({ name: "SHA-256" }, arrayBuffer);

  // Convert hash buffer to Base64 (manual conversion to avoid undio edge issues)
  const hashArray = new Uint8Array(hashBuffer);
  const binaryString = String.fromCharCode(...hashArray.values());
  const base64 = btoa(binaryString);

  // Return complete SRI format
  return `sha256-${base64}`;
}
