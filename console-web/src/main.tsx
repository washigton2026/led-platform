import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";

const raiz = document.getElementById("root");
if (!raiz) throw new Error("elemento #root ausente no index.html");

createRoot(raiz).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
