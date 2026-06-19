import { Navigate, Route, Routes } from "react-router-dom";
import { useApp } from "./app/AppContext";
import { Layout } from "./app/Layout";
import { Login } from "./pages/Login";
import { Setup } from "./pages/Setup";
import { Issue } from "./pages/Issue";
import { Records } from "./pages/Records";
import { ImportFromUser } from "./pages/ImportFromUser";
import { Verify } from "./pages/Verify";
import { Settings } from "./pages/Settings";

export function App() {
  const { opToken } = useApp();

  if (!opToken) return <Login />;

  return (
    <Routes>
      <Route path="/setup" element={<Layout title="Setup"><Setup /></Layout>} />
      <Route path="/issue" element={<Layout title="Issue credential"><Issue /></Layout>} />
      <Route path="/records" element={<Layout title="Records"><Records /></Layout>} />
      <Route path="/import" element={<Layout title="Import from user"><ImportFromUser /></Layout>} />
      <Route path="/verify" element={<Layout title="Export"><Verify /></Layout>} />
      <Route path="/settings" element={<Layout title="Settings"><Settings /></Layout>} />
      <Route path="*" element={<Navigate to="/issue" replace />} />
    </Routes>
  );
}
