# Odysync app

Tauri 2 shell (`src-tauri/`) around a React 19 + TypeScript front-end (`src/`),
styled with Tailwind 4.

- `src/api.ts` — one function per Tauri command; `src/bridge.ts` routes calls to
  Tauri, or to the demo data in `src/demo/` when running in a plain browser.
- `src/store.tsx` — every resource the pages show, loaded once in the
  background and refreshed on demand.
- `src/components/ui.tsx` — the shared building blocks; colours are semantic
  tokens defined for both themes in `src/index.css`.
- `src/pages/` — one file per page.

```sh
npm ci
npm run dev          # browser, demo data; add ?theme=light or ?theme=dark
npx tauri dev        # the real app
npm test             # Vitest + Testing Library
npx tauri build --config src-tauri/release-windows.conf.json   # or release-unix
```
