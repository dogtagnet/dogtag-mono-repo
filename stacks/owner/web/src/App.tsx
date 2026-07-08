import { NavLink, Navigate, Route, Routes } from "react-router-dom";
import { useWallet, shortAddr } from "./lib/hooks";
import type { OwnerWallet } from "./lib/wallet";
import { Wallet } from "./pages/Wallet";
import { Receive } from "./pages/Receive";
import { CredentialDetail } from "./pages/CredentialDetail";
import { Present } from "./pages/Present";
import { Share } from "./pages/Share";
import { Receipt } from "./pages/Receipt";
import { Receipts } from "./pages/Receipts";

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
        <NavLink to="/present" className={({ isActive }) => (isActive ? "active" : "")}>
          Present
        </NavLink>
      </nav>
      <Routes>
        <Route path="/" element={<Navigate to="/wallet" replace />} />
        <Route path="/wallet" element={<Wallet />} />
        <Route path="/receive" element={<Receive />} />
        <Route path="/receipts" element={<Receipts />} />
        <Route path="/credential/:id" element={<CredentialDetail wallet={wallet} />} />
        <Route path="/receipt/:id" element={<Receipt />} />
        <Route path="/share/:id" element={<Share />} />
        <Route path="/present" element={<Present wallet={wallet} />} />
        <Route path="*" element={<Navigate to="/wallet" replace />} />
      </Routes>
      <p className="foot">
        Your credentials and keys live only on this device. Proofs are generated with your own key;
        verifiers learn only that a valid credential exists — never its contents.
      </p>
    </div>
  );
}
