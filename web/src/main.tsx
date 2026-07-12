import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// Real, self-hosted JetBrains Mono webfont (OFL-licensed, via the
// official @fontsource package) -- see desktop/src/main.tsx's own
// identical wiring; both shells share theme.css's real .mono rule.
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "./theme.css";
import "./app.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
