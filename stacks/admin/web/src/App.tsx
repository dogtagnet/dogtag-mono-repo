import { Navigate, Route, Routes } from "react-router-dom";
import { useApp } from "./app/AppContext";
import { Layout } from "./app/Layout";
import { Login } from "./pages/Login";
import { Dashboard } from "./pages/Dashboard";
import { Activity } from "./pages/Activity";
import { Issuers } from "./pages/Issuers";
import { Providers } from "./pages/Providers";
import { Businesses } from "./pages/Businesses";
import { IssuerApplications } from "./pages/IssuerApplications";
import { Whitelist } from "./pages/Whitelist";
import { Wizard } from "./pages/Wizard";
import { Governance } from "./pages/Governance";
import { VerificationBench } from "./pages/VerificationBench";
import { Settings } from "./pages/Settings";

export function App() {
  const { adminToken } = useApp();

  if (!adminToken) return <Login />;

  return (
    <Routes>
      <Route path="/dashboard" element={<Layout title="Dashboard"><Dashboard /></Layout>} />
      <Route path="/activity" element={<Layout title="On-chain activity"><Activity /></Layout>} />
      <Route path="/issuers" element={<Layout title="Issuers / Factory"><Issuers /></Layout>} />

      <Route path="/providers" element={<Layout title="Providers"><Providers /></Layout>} />

      <Route path="/onboard" element={<Layout title="Onboard issuer"><Wizard /></Layout>} />
      <Route path="/businesses" element={<Layout title="Business registry"><Businesses /></Layout>} />
      <Route
        path="/applications"
        element={<Layout title="Issuer applications"><IssuerApplications /></Layout>}
      />
      <Route path="/whitelist" element={<Layout title="Whitelist viewer"><Whitelist /></Layout>} />
      <Route path="/governance" element={<Layout title="Governance"><Governance /></Layout>} />
      <Route
        path="/bench"
        element={<Layout title="Verification bench"><VerificationBench /></Layout>}
      />
      <Route path="/settings" element={<Layout title="Settings"><Settings /></Layout>} />
      <Route path="*" element={<Navigate to="/dashboard" replace />} />
    </Routes>
  );
}
