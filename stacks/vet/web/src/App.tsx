import { Navigate, Route, Routes } from "react-router-dom";
import { useApp } from "./app/AppContext";
import { Layout } from "./app/Layout";
import { Login } from "./pages/Login";
import { Setup } from "./pages/Setup";
import { Unlock } from "./pages/Unlock";
import { Issue } from "./pages/Issue";
import { IssueDogTag } from "./pages/IssueDogTag";
import { Records } from "./pages/Records";
import { Traceability } from "./pages/Traceability";
import { ImportFromUser } from "./pages/ImportFromUser";
import { Verify } from "./pages/Verify";
import ProviderSelfService from "./pages/ProviderSelfService";
import { Settings } from "./pages/Settings";

export function App() {
  const { opToken } = useApp();

  if (!opToken) return <Login />;

  return (
    <Routes>
      {/* Standalone (no Layout): unlocking is the whole page, not a section of the portal. */}
      <Route path="/unlock" element={<Unlock />} />
      <Route path="/setup" element={<Layout title="Setup"><Setup /></Layout>} />
      {/* Register-first: registering a pet is the prerequisite, so it leads (see Layout's NAV). */}
      <Route
        path="/issue-dog-tag"
        element={<Layout title="Register pet (issue dog tag)"><IssueDogTag /></Layout>}
      />
      <Route path="/issue" element={<Layout title="Issue a record (e.g. vaccination)"><Issue /></Layout>} />
      <Route path="/records" element={<Layout title="Records"><Records /></Layout>} />
      <Route path="/traceability" element={<Layout title="Traceability"><Traceability /></Layout>} />
      <Route path="/import" element={<Layout title="Import from user"><ImportFromUser /></Layout>} />
      <Route path="/verify" element={<Layout title="Verification"><Verify /></Layout>} />
      <Route
        path="/provider"
        element={<Layout title="Provider self-service"><ProviderSelfService /></Layout>}
      />
      <Route path="/settings" element={<Layout title="Settings"><Settings /></Layout>} />
      <Route path="*" element={<Navigate to="/issue" replace />} />
    </Routes>
  );
}
