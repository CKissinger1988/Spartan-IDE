import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// Real, self-hosted JetBrains Mono webfont (OFL-licensed, via the
// official @fontsource package) -- loaded before theme.css so its
// @font-face declarations are registered by the time anything with the
// "mono" class first paints. See theme.css's own .mono rule for why this
// is now the primary font, not a fallback in the stack.
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "./theme.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
