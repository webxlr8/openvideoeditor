import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { ErrorBoundary, logRenderError } from "./ErrorBoundary";
import { reportError } from "./notify";
import "./styles.css";

// Last line of defence for async work that slipped past `fire` / `runCommand`: say so instead of failing silently.
window.addEventListener("unhandledrejection", (e) => {
  e.preventDefault();
  reportError(e.reason);
});

createRoot(document.getElementById("root")!, {
  onCaughtError: (error, info) => logRenderError("caught", error, info),
  onUncaughtError: (error, info) => logRenderError("uncaught", error, info),
}).render(
  <StrictMode>
    {/* Regions have their own boundaries; this one keeps a blank window from ever being the result. */}
    <ErrorBoundary
      region="OpenVideoEditor"
      className="app"
      reloadLabel="Reload the window"
    >
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
