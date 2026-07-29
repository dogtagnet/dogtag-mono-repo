import { NavLink, Navigate, Route, Routes } from "react-router-dom";
import { useWallet, shortAddr } from "./lib/hooks";
import type { OwnerWallet } from "./lib/wallet";
import { Wallet } from "./pages/Wallet";
import { Receive } from "./pages/Receive";
import { CredentialDetail } from "./pages/CredentialDetail";
import { Share } from "./pages/Share";
import { Receipt } from "./pages/Receipt";
import { Receipts } from "./pages/Receipts";
import { Consents } from "./pages/Consents";
import { ConsentDetail } from "./pages/ConsentDetail";
import { Settings } from "./pages/Settings";

function TopBar({ wallet }: { wallet: OwnerWallet | null }) {
  return (
    <div className="topbar">
      <div className="brand">
        <span className="paw" aria-hidden>
          🐾
        </span>
        <span>
          DogTag Wallet
          <small>Pet owner · holder</small>
        </span>
      </div>
      <span className="live-badge">LIVE · ROAX</span>
      <div className="owner-chip" title={wallet?.address ?? "preparing wallet"} data-testid="owner-address">
        <span className="dot" />
        {wallet ? shortAddr(wallet.address) : "preparing…"}
      </div>
    </div>
  );
}

export function App() {
  const wallet = useWallet();
  return (
    <div className="shell">
      <TopBar wallet={wallet} />
      <nav className="tabs">
        <NavLink to="/wallet" className={({ isActive }) => (isActive ? "active" : "")}>
          My wallet
        </NavLink>
        <NavLink to="/receive" className={({ isActive }) => (isActive ? "active" : "")}>
          Receive
        </NavLink>
        <NavLink to="/receipts" className={({ isActive }) => (isActive ? "active" : "")}>
          Receipts
        </NavLink>
        <NavLink to="/consents" className={({ isActive }) => (isActive ? "active" : "")}>
          Consents
        </NavLink>
        <NavLink to="/settings" className={({ isActive }) => (isActive ? "active" : "")}>
          Settings
        </NavLink>
      </nav>
      <Routes>
        <Route path="/" element={<Navigate to="/wallet" replace />} />
        <Route path="/wallet" element={<Wallet />} />
        <Route path="/receive" element={<Receive />} />
        <Route path="/receipts" element={<Receipts />} />
        <Route path="/consents" element={<Consents />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="/consents/:nullifier" element={<ConsentDetail />} />
        <Route path="/credential/:id" element={<CredentialDetail />} />
        <Route path="/receipt/:id" element={<Receipt />} />
        <Route path="/share/:id" element={<Share />} />
        <Route path="*" element={<Navigate to="/wallet" replace />} />
      </Routes>
      <p className="foot">
        Your credentials and keys live only on this device. You decide which credential fields to
        share.
      </p>
    </div>
  );
}
