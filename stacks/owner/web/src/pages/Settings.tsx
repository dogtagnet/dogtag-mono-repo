import { useRoaxRpcSettings } from "@dogtag/ui/rpc";
import type { FormEvent } from "react";
import { ROAX_RPC_URL } from "../lib/config";

export function Settings() {
  const settings = useRoaxRpcSettings(ROAX_RPC_URL);

  function save(event: FormEvent) {
    event.preventDefault();
    void settings.save();
  }

  return (
    <>
      <div className="section-title">
        <h1>Settings</h1>
      </div>
      <div className="card">
        <h2>Blockchain endpoint</h2>
        <p className="sub">
          Choose the JSON-RPC peer this wallet uses for gasless ROAX reads. A custom endpoint can
          improve availability and help when one endpoint censors requests.
        </p>
        <form onSubmit={save}>
          <label htmlFor="owner-roax-rpc">ROAX JSON-RPC URL</label>
          <input
            id="owner-roax-rpc"
            type="url"
            inputMode="url"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            value={settings.draft}
            disabled={settings.checking}
            onChange={(event) => settings.setDraft(event.target.value)}
            placeholder="https://rpc.example"
          />
          <div className="btn-row">
            <button className="btn" type="submit" disabled={settings.checking}>
              {settings.checking ? "Checking…" : "Check and save"}
            </button>
            <button
              className="btn secondary"
              type="button"
              onClick={settings.reset}
              disabled={
                settings.checking ||
                (!settings.isCustom && settings.draft === settings.defaultRpcUrl)
              }
            >
              Restore default
            </button>
          </div>
        </form>
        {settings.message &&
          (settings.message.tone === "danger" ? (
            <p className="error-box" role="alert">
              {settings.message.text}
            </p>
          ) : (
            <p className="notice" role="status">
              {settings.message.text}
            </p>
          ))}
      </div>

      <div className="card rpc-trust-warning">
        <h2>Endpoint choice is not a trust upgrade</h2>
        <p className="sub">
          This is plain JSON-RPC, not a light client. Any endpoint can fabricate{" "}
          <code>isValid</code>, <code>rootIssuer</code>, <code>profileRoot</code>, logs, or
          transaction data. The chain guard only prevents DogTag from using its bundled contract
          addresses on a different chain.
        </p>
        <p className="sub">
          Centralized app APIs and the provider directory/indexer stay fixed and are not changed by
          this setting.
        </p>
        <p className="rpc-default">
          Bundled default: <code>{settings.defaultRpcUrl}</code>
        </p>
      </div>
    </>
  );
}
