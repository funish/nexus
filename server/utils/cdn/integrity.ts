/**
 * Calculate SHA-256 hash for Subresource Integrity (SRI)
 *
 * Uses Bun's native CryptoHasher for optimized SHA-256 calculation.
 * Directly accepts Uint8Array without requiring string conversion.
 *
 * @param data - File data as Uint8Array
 * @returns SRI integrity string in format "sha256-{base64-hash}"
 */
export async function calculateIntegrity(data: Uint8Array): Promise<string> {
  const hasher = new Bun.CryptoHasher("sha256");
  hasher.update(data);
  return `sha256-${hasher.digest("base64")}`;
}
