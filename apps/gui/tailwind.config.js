/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      fontFamily: {
        mono: ['"JetBrains Mono"', '"Fira Code"', "monospace"],
        sans: ['"JetBrains Mono"', '"Inter"', "system-ui", "sans-serif"],
      },
      colors: {
        accent: {
          DEFAULT: "#00f0ff",
          hover: "#00d4e6",
          dim: "#00a8b5",
        },
        purple: {
          neon: "#a855f7",
          dim: "#7c3aed",
        },
        cyber: {
          bg: "#0a0a0f",
          surface: "#13131a",
          "surface-2": "#1a1a24",
          border: "#1e1e2a",
          "border-bright": "#2a2a3e",
          text: "#e0e0e8",
          "text-dim": "#8888a0",
          "text-faint": "#555568",
        },
        success: "#22c55e",
        warning: "#f59e0b",
        danger: "#ef4444",
      },
      animation: {
        "glow-pulse": "glow-pulse 2s ease-in-out infinite",
        "spin-slow": "spin 3s linear infinite",
        "fade-in": "fade-in 0.3s ease-out",
        "slide-up": "slide-up 0.3s ease-out",
        "scan-line": "scan-line 1.5s ease-in-out infinite",
      },
      keyframes: {
        "glow-pulse": {
          "0%, 100%": { boxShadow: "0 0 8px rgba(0, 240, 255, 0.3)" },
          "50%": { boxShadow: "0 0 20px rgba(0, 240, 255, 0.6)" },
        },
        "fade-in": {
          "0%": { opacity: "0" },
          "100%": { opacity: "1" },
        },
        "slide-up": {
          "0%": { opacity: "0", transform: "translateY(10px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        "scan-line": {
          "0%, 100%": { transform: "translateX(-100%)" },
          "50%": { transform: "translateX(100%)" },
        },
      },
    },
  },
  plugins: [],
};
