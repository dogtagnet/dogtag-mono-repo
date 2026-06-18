import type { RecordStatus } from "@dogtag/ui";
import { useCallback, useEffect, useState } from "react";

/**
 * Local record index. The groomer backend (routes.rs, identical to vet) exposes no "list records"
 * endpoint — records are addressed by id (share/revoke/get). The portal therefore keeps a local
 * index of records it has issued this session/device so the issue flow has something to render.
 */
export interface LocalRecord {
  recordId: string;
  recordType: string;
  dogTagId: string;
  merkleRoot: string;
  txHash?: string;
  status: RecordStatus;
  createdAt: number;
}

const KEY = "groomer.records";

function read(): LocalRecord[] {
  try {
    const raw = window.localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as LocalRecord[]) : [];
  } catch {
    return [];
  }
}

function write(rows: LocalRecord[]) {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(rows));
  } catch {
    /* ignore */
  }
}

export function useRecordsStore() {
  const [records, setRecords] = useState<LocalRecord[]>(() => read());

  useEffect(() => {
    write(records);
  }, [records]);

  const upsert = useCallback((rec: LocalRecord) => {
    setRecords((prev) => {
      const next = prev.filter((r) => r.recordId !== rec.recordId);
      return [rec, ...next];
    });
  }, []);

  const setStatus = useCallback((recordId: string, status: RecordStatus, txHash?: string) => {
    setRecords((prev) =>
      prev.map((r) => (r.recordId === recordId ? { ...r, status, txHash: txHash ?? r.txHash } : r)),
    );
  }, []);

  return { records, upsert, setStatus };
}
