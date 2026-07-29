import { useCallback, useEffect, useRef, useState } from "react";
import {
  DEFAULT_ROAX_RPC_URL,
  RpcPreferenceOperationSupersededError,
  getRoaxRpcPreferenceRevision,
  validateAndSaveRoaxRpcPreference,
} from "./rpcEndpoint";
import { useRoaxRpcPreference } from "./useRoaxRpcPreference";

export interface RoaxRpcSettingsMessage {
  tone: "success" | "danger";
  text: string;
}

/**
 * Shared endpoint-settings state machine.
 *
 * Operation generation handles Reset/unmount; the preference revision handles same-tab and
 * cross-tab changes. Both are checked inside validation immediately before persistence, not merely
 * after the promise resolves, so a stale completion cannot overwrite the newer choice.
 */
export function useRoaxRpcSettings(
  defaultRpcUrl: string = DEFAULT_ROAX_RPC_URL,
) {
  const preference = useRoaxRpcPreference(defaultRpcUrl);
  const [draft, setDraft] = useState(preference.rpcUrl);
  const [checking, setChecking] = useState(false);
  const [message, setMessage] = useState<RoaxRpcSettingsMessage>();
  const operation = useRef(0);
  const mounted = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      operation.current += 1;
    };
  }, []);

  useEffect(() => {
    // This also cancels a probe when a storage event changes the preference in another tab.
    operation.current += 1;
    setDraft(preference.rpcUrl);
    setChecking(false);
  }, [preference.rpcUrl]);

  const save = useCallback(async () => {
    const thisOperation = ++operation.current;
    const expectedRevision = getRoaxRpcPreferenceRevision();
    const isCurrent = () =>
      mounted.current && operation.current === thisOperation;
    setMessage(undefined);
    setChecking(true);
    try {
      const saved = await validateAndSaveRoaxRpcPreference(draft, {
        defaultUrl: preference.defaultRpcUrl,
        expectedRevision,
        shouldApply: isCurrent,
      });
      if (!isCurrent()) return;
      setDraft(saved.rpcUrl);
      setMessage({
        tone: "success",
        text: saved.isCustom
          ? "Custom endpoint saved and confirmed on ROAX chain 135."
          : "Using the bundled endpoint for ROAX chain 135.",
      });
    } catch (cause) {
      if (cause instanceof RpcPreferenceOperationSupersededError || !isCurrent()) return;
      setDraft(preference.defaultRpcUrl);
      setMessage({
        tone: "danger",
        text: cause instanceof Error ? cause.message : "The endpoint could not be checked.",
      });
    } finally {
      if (isCurrent()) setChecking(false);
    }
  }, [draft, preference.defaultRpcUrl]);

  const reset = useCallback(() => {
    operation.current += 1;
    const next = preference.reset();
    setChecking(false);
    setDraft(next.rpcUrl);
    setMessage({
      tone: "success",
      text: "Custom endpoint removed. Blockchain reads use the bundled default.",
    });
  }, [preference.reset]);

  return {
    ...preference,
    draft,
    setDraft,
    checking,
    message,
    save,
    reset,
  };
}
