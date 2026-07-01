/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        sans: ["Plus Jakarta Sans", "ui-sans-serif", "system-ui", "sans-serif"],
        display: [
          "Plus Jakarta Sans",
          "ui-sans-serif",
          "system-ui",
          "sans-serif",
        ],
      },
      colors: {
        // New editorial palette
        canvas: "var(--color-canvas)",
        "canvas-soft": "var(--color-canvas-soft)",
        surface: "var(--color-surface)",
        "surface-strong": "var(--color-surface-strong)",
        ink: "var(--color-ink)",
        "ink-soft": "var(--color-ink-soft)",
        body: "var(--color-body)",
        muted: "var(--color-muted)",
        "muted-soft": "var(--color-muted-soft)",
        hairline: "var(--color-hairline)",
        "hairline-strong": "var(--color-hairline-strong)",
        "on-primary": "var(--color-on-primary)",
        "toggle-track-on": "var(--color-toggle-track-on)",
        "toggle-knob-on": "var(--color-toggle-knob-on)",
        "orb-mint": "var(--color-orb-mint)",
        "orb-peach": "var(--color-orb-peach)",
        "orb-lavender": "var(--color-orb-lavender)",
        "orb-sky": "var(--color-orb-sky)",
        "orb-rose": "var(--color-orb-rose)",
        success: "var(--color-success)",
        error: "var(--color-error)",

        // Back-compat aliases (still referenced by leaf components)
        text: "var(--color-text)",
        background: "var(--color-background)",
        "background-ui": "var(--color-background-ui)",
        "logo-primary": "var(--color-logo-primary)",
        "logo-stroke": "var(--color-logo-stroke)",
        "text-stroke": "var(--color-text-stroke)",
        "mid-gray": "var(--color-mid-gray)",
      },
    },
  },
  plugins: [],
};
