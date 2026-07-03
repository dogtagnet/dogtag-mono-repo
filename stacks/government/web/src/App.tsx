import { useEffect, useState } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "./app/Layout";
import { apiGet, type Health } from "./lib/api";
import { Issue } from "./pages/Issue";
import { Receipt } from "./pages/Receipt";
import { Records } from "./pages/Records";
import { Verify } from "./pages/Verify";

export function App() {
  const [health, setHealth] = useState<Health | null>(null);
  useEffect(() => {
    apiGet("/health")
      .then(setHealth)
      .catch(() => setHealth(null));
  }, []);

  return (
    <Routes>
      <Route path="/" element={<Navigate to="/issue" replace />} />
      <Route
        path="/issue"
        element={
          <Layout title="Issue" health={health}>
            <Issue />
          </Layout>
        }
      />
      <Route
        path="/verify"
        element={
          <Layout title="Verify" health={health}>
            <Verify health={health} />
          </Layout>
        }
      />
      <Route
        path="/records"
        element={
          <Layout title="Records" health={health}>
            <Records />
          </Layout>
        }
      />
      <Route
        path="/receipt/:root"
        element={
          <Layout title="Travel-clearance receipt" health={health}>
            <Receipt />
          </Layout>
        }
      />
      <Route path="*" element={<Navigate to="/issue" replace />} />
    </Routes>
  );
}
