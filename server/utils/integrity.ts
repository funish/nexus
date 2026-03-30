/**
 * Calculate SHA-256 hash for Subresource Integrity (SRI)
 *
 * Uses Bun's native CryptoHasher for optimized SHA-256 calculation.
 * Directly accepts Uint8Array without requiring string conversion.
 *
 * @param data - File data as Uint8Array
 * @returns SRI integrity string in format "sha256-{base64-hash}"
 *
 * @example
 * ```typescript
 * const integrity = await calculateIntegrity(fileData);
 * // Returns: "sha256-hwg4gsxgFZhOsEEamdOYGBf13FyQuiTwlAQgxVSNgt4="
 * ```
 */
export async function calculateIntegrity(data: Uint8Array): Promise<string> {
  // Create SHA-256 hasher
  const hasher = new Bun.CryptoHasher("sha256");

  // Update with data (can accept Uint8Array directly)
  hasher.update(data);

  // Get hash as Uint8Array
  const hash = hasher.digest();

  // Convert Uint8Array to binary string, then to base64
  const binaryString = String.fromCharCode(...hash);
  const base64 = btoa(binaryString);

  // Return complete SRI format
  return `sha256-${base64}`;
}
