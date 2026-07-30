import { useCallback, useSyncExternalStore } from "react";
import {
  DEFAULT_ROAX_RPC_URL,
  getRoaxRpcPreference,
  resetRoaxRpcPreference,
  setRoaxRpcPreference,
  subscribeRoaxRpcPreference,
  type RoaxRpcPreference,
} from "./rpcEndpoint";

/** Reactive view of the browser-local chain endpoint choice. */
export function useRoaxRpcPreference(
  defaultRpcUrl: string = DEFAULT_ROAX_RPC_URL,
): RoaxRpcPreference & {
  save: (value: string) => RoaxRpcPreference;
  reset: () => RoaxRpcPreference;
} {
  const subscribe = useCallback(
    (listener: () => void) => subscribeRoaxRpcPreference(listener),
    [],
  );
  const getSnapshot = useCallback(
    () => JSON.stringify(getRoaxRpcPreference(defaultRpcUrl)),
    [defaultRpcUrl],
  );
  // useSyncExternalStore snapshots must be referentially stable. A serialized primitive is stable
  // when localStorage is unchanged; returning getRoaxRpcPreference's fresh object here loops.
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const preference = JSON.parse(snapshot) as RoaxRpcPreference;
  const save = useCallback(
    (value: string) => setRoaxRpcPreference(value, defaultRpcUrl),
    [defaultRpcUrl],
  );
  const reset = useCallback(
    () => resetRoaxRpcPreference(defaultRpcUrl),
    [defaultRpcUrl],
  );
  return { ...preference, save, reset };
}
