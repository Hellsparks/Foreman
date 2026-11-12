import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./design/tokens.css";
import "./theme/theme.css";
import "./ui/ui.css";
import "./ui/phone.css";
import "./ui/layout.css";
import App from "./App";
import { initUserTheme } from "./theme/userTheme";

// Apply appearance from localStorage synchronously before React renders to
// prevent a flash of the default (Fieldwork light) scheme.  The defaults here
// must match the defaults in useAppearance.ts.
const root = document.documentElement;
root.setAttribute(
  "data-fw-theme",
  localStorage.getItem("fw-theme") ?? "fieldwork"
);
root.setAttribute(
  "data-fw-mode",
  localStorage.getItem("fw-mode") ?? "dark"
);

// User theme file + custom.css (Settings → Appearance), after the scheme.
initUserTheme();

const container = document.getElementById("root");
if (!container) throw new Error("Root element #root not found");

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>
);
