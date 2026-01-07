import { digest } from "ohash";

/**
 * Calculate SHA-256 hash for Subresource Integrity (SRI)
 *
 * Uses ohash for optimized SHA-256 calculation with proper base64 encoding
 * to ensure compatibility across all environments including edge workers.
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
  // Convert Uint8Array to string for ohash
  const decoder = new TextDecoder();
  const text = decoder.decode(data);

  // Use ohash for optimized SHA-256 (returns base64url format)
  const base64url = digest(text);

  // Convert base64url to standard base64 for SRI compatibility
  // Replace: - -> +, _ -> /, add padding
  const base64 = base64url
    .replace(/-/g, "+")
    .replace(/_/g, "/")
    .padEnd(base64url.length + ((4 - (base64url.length % 4)) % 4), "=");

  // Return complete SRI format
  return `sha256-${base64}`;
}
