/** @type {import('tailwindcss').Config} */
// Tailwind config for the extension popup + options page. Mirrors the
// webapp's design tokens (`webapp/tailwind.config.js`) so the popup and
// the main app share one dark-theme palette. Keeping a separate config
// here lets the extension build avoid pulling webapp source into its
// content-scan globs and keeps the popup CSS bundle minimal.
export default {
  content: [
    "./src/popup/**/*.{ts,tsx,html}",
    "./src/options/**/*.{ts,tsx,html}",
  ],
  theme: {
    extend: {
      colors: {
        background: "#0A0F1E",
        "accent-primary": "#00D4B4",
        "accent-secondary": "#9945FF",
        "text-primary": "#FFFFFF",
        "text-muted": "#8B9BC0",
        error: "#FF4747",
        success: "#00CC88",
      },
      fontFamily: {
        mono: [
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "Monaco",
          "Consolas",
          "Liberation Mono",
          "Courier New",
          "monospace",
        ],
      },
    },
  },
  plugins: [],
};
