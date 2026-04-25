/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
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
