/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  groomerApiBase: import.meta.env.VITE_GROOMER_API_BASE ?? "/api",
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "http://localhost:39742",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
};
