/**
 * Get content type for a file using Bun's native MIME type detection
 *
 * Uses Bun.file() to detect MIME type based on file extension.
 * Bun automatically includes charset for text-based content types.
 *
 * @param filepath - The file path to lookup
 * @returns Content type with charset if applicable
 *
 * @example
 * getContentType("index.js") // "text/javascript;charset=utf-8"
 * getContentType("image.png") // "image/png"
 */
export function getContentType(filepath: string): string {
  return Bun.file(filepath).type;
}
