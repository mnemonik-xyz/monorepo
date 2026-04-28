/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:3000",
        changeOrigin: true,
      },
      "/chat": {
        target: "http://localhost:3000",
        changeOrigin: true,
        // The chat API is POST /chat, but `/chat` is also a SPA route now.
        // Bypass the proxy for non-POST requests so GET /chat falls through to
        // Vite's index.html middleware and the React Router renders the page.
        bypass: (req) => {
          if (req.method !== "POST") {
            return req.url;
          }
          return undefined;
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: true,
    // Playwright e2e specs live under webapp/e2e/ and must not be run by Vitest.
    exclude: ["node_modules/**", "dist/**", "e2e/**", ".idea/**", ".git/**"],
  },
});
