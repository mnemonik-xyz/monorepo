/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        background: "#080b16",
        ink: "#080b16",
        panel: "#0d1322",
        "panel-raised": "#121a2c",
        "accent-primary": "#00d4b4",
        "accent-secondary": "#9945ff",
        stamp: "#ff5c38",
        paper: "#f4ead9",
        "text-primary": "#f3f6fb",
        "text-muted": "#8b9bc0",
        "text-faint": "#56648a",
        error: "#ff4747",
        success: "#00cc88",
      },
      fontFamily: {
        display: [
          "Charter",
          "Iowan Old Style",
          "Newsreader",
          "ui-serif",
          "Georgia",
          "Times New Roman",
          "serif",
        ],
        sans: [
          "Avenir Next",
          "Hanken Grotesk",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "sans-serif",
        ],
        mono: [
          "SFMono-Regular",
          "ui-monospace",
          "Cascadia Code",
          "JetBrains Mono",
          "Menlo",
          "Consolas",
          "monospace",
        ],
      },
    },
  },
  plugins: [],
};
