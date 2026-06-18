import { ThemeProvider, ToastProvider, WalletProvider } from "@dogtag/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { App } from "./App";
import { AppProvider } from "./app/AppContext";
import { env } from "./lib/env";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");

createRoot(root).render(
  <StrictMode>
    <ThemeProvider defaultTheme="light">
      <WalletProvider
        options={{
          projectId: env.reownProjectId,
          appName: "DogTag Vet Portal",
          appDescription: "Issue and verify pet credentials on ROAX",
          appUrl: env.deploymentUrl,
        }}
      >
        <ToastProvider>
          <AppProvider>
            <BrowserRouter>
              <App />
            </BrowserRouter>
          </AppProvider>
        </ToastProvider>
      </WalletProvider>
    </ThemeProvider>
  </StrictMode>,
);
