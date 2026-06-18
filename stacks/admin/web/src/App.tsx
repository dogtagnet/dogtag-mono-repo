import { Navigate, Route, Routes } from "react-router-dom";
import { useApp } from "./app/AppContext";
import { Layout } from "./app/Layout";
import { Login } from "./pages/Login";
import { Dashboard } from "./pages/Dashboard";
import { Businesses } from "./pages/Businesses";
import { IssuerApplications } from "./pages/IssuerApplications";
import { Whitelist } from "./pages/Whitelist";
import { Wizard } from "./pages/Wizard";

export function App() {
  const { adminToken } = useApp();

  if (!adminToken) return <Login />;

  return (
    <Routes>
      <Route path="/dashboard" element={<Layout title="Dashboard"><Dashboard /></Layout>} />
      <Route path="/onboard" element={<Layout title="Onboard issuer"><Wizard /></Layout>} />
      <Route path="/businesses" element={<Layout title="Business registry"><Businesses /></Layout>} />
      <Route
        path="/applications"
        element={<Layout title="Issuer applications"><IssuerApplications /></Layout>}
      />
      <Route path="/whitelist" element={<Layout title="Whitelist viewer"><Whitelist /></Layout>} />
      <Route path="*" element={<Navigate to="/dashboard" replace />} />
    </Routes>
  );
}
