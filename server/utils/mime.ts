import { mimes } from "mrmime";

/**
 * Add UTF-8 charset to text-based content types
 *
 * @param contentType - The MIME type to check
 * @returns Content type with charset appended if text-based
 *
 * @example
 * addCharsetToContentType("text/plain") // "text/plain; charset=utf-8"
 * addCharsetToContentType("application/json") // "application/json; charset=utf-8"
 * addCharsetToContentType("application/pdf") // "application/pdf"
 */
export function addCharsetToContentType(contentType: string): string {
  // Text types
  if (contentType.startsWith("text/")) {
    return `${contentType}; charset=utf-8`;
  }

  // Specific application types that are text-based
  const textBasedApplicationTypes = [
    "application/json",
    "application/javascript",
    "application/xml",
    "application/xhtml+xml",
    "application/x-www-form-urlencoded",
  ];

  if (textBasedApplicationTypes.includes(contentType)) {
    return `${contentType}; charset=utf-8`;
  }

  // Binary types - no charset
  return contentType;
}

/**
 * Custom MIME type lookup with corrected types
 *
 * @param filepath - The file path to lookup
 * @returns The MIME type for the file
 */
export function lookup(filepath: string): string {
  const ext = (filepath.split(".").pop() || "").toLowerCase();

  // Override incorrect types from mrmime
  if (ext === "ts" || ext === "tsx") {
    return "text/typescript";
  }

  // Use mrmime's lookup for other types
  return mimes[ext] || "application/octet-stream";
}

/**
 * Get content type for a file with UTF-8 charset for text files
 *
 * @param filepath - The file path to lookup
 * @returns Content type with charset if applicable
 *
 * @example
 * getContentType("index.js") // "text/javascript; charset=utf-8"
 * getContentType("image.png") // "image/png"
 */
export function getContentType(filepath: string): string {
  const contentType = lookup(filepath);
  return addCharsetToContentType(contentType);
}
