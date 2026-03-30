import { defineMiddleware, handleCors } from "nitro/h3";

/**
 * CORS middleware for all routes
 * Enables Cross-Origin Resource Sharing for public API access
 */
export default defineMiddleware((event) => {
  handleCors(event, {
    origin: "*",
    methods: "*",
  });
});
