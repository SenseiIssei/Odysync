import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App.tsx";
import { reportFrontendError } from "./api";

// A release webview has no console the user can open, so anything that escapes
// React's error boundaries is forwarded to odysync.log instead of vanishing.
window.addEventListener("error", (e) => {
  void reportFrontendError("window.onerror", e.error?.stack ?? e.message);
});

window.addEventListener("unhandledrejection", (e) => {
  const reason: unknown = e.reason;
  void reportFrontendError(
    "unhandledrejection",
    reason instanceof Error ? (reason.stack ?? reason.message) : String(reason),
  );
});

const root = document.getElementById("root");
if (!root) {
  throw new Error("index.html is missing its #root element");
}

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
