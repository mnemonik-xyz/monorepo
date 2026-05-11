// PostCSS config for the extension. Tailwind v3 + autoprefixer; matches
// the toolchain used by the webapp so the design-token classes resolve
// identically in both builds.
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
