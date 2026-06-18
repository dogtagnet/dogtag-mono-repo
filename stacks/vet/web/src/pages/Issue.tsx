import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  QrCode,
  RECORD_TYPE_SCHEMAS,
  ROAX_CHAIN_ID,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  buildFieldsObject,
  explorerTxUrl,
  schemaFor,
  useToast,
  validateField,
  type FieldDef,
  type PrepareResp,
} from "@dogtag/ui";
import { CheckCircle2 } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { useSendTransaction } from "wagmi";
import { useApp } from "../app/AppContext";
import { useRecordsStore } from "../app/recordsStore";

export function Issue() {
  const { api, signingMode } = useApp();
  const { toast } = useToast();
  const { upsert } = useRecordsStore();
  const { sendTransactionAsync } = useSendTransaction();

  const [recordType, setRecordType] = useState(RECORD_TYPE_SCHEMAS[0]?.recordType ?? "");
  const [dogTagId, setDogTagId] = useState("");
  const [values, setValues] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [issued, setIssued] = useState<{ resp: PrepareResp; txHash?: string } | null>(null);
  const [shareUrl, setShareUrl] = useState<string | null>(null);

  const schema = useMemo(() => schemaFor(recordType), [recordType]);
  const allFields = useMemo(
    () => (schema ? schema.groups.flatMap((g) => g.fields) : []),
    [schema],
  );

  function setVal(path: string, v: string) {
    setValues((prev) => ({ ...prev, [path]: v }));
    setErrors((prev) => {
      const { [path]: _drop, ...rest } = prev;
      return rest;
    });
  }

  function validate(): boolean {
    const next: Record<string, string> = {};
    if (!dogTagId.trim()) next["__dogTagId"] = "dogTagId is required";
    for (const f of allFields) {
      const err = validateField(f, values[f.path] ?? "");
      if (err) next[f.path] = err;
    }
    setErrors(next);
    return Object.keys(next).length === 0;
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!validate()) {
      toast({ title: "Please fix the highlighted fields", variant: "danger" });
      return;
    }
    setBusy(true);
    try {
      const fields = buildFieldsObject(values);
      const prep = await api.prepare({ recordType, dogTagId: dogTagId.trim(), fields });

      let txHash = prep.txHash;

      // wallet mode → the connected wallet signs + broadcasts the unsignedTx, then we confirm.
      if (prep.unsignedTx) {
        const hash = await sendTransactionAsync({
          to: prep.unsignedTx.to as `0x${string}`,
          data: prep.unsignedTx.data as `0x${string}`,
          value: BigInt(prep.unsignedTx.value),
          chainId: prep.unsignedTx.chainId ?? ROAX_CHAIN_ID,
        });
        await api.confirm({ recordId: prep.recordId, txHash: hash });
        txHash = hash;
      }
      // backend mode → prepare already broadcast + confirmed; txHash is on the response.

      upsert({
        recordId: prep.recordId,
        recordType,
        dogTagId: dogTagId.trim(),
        merkleRoot: prep.merkleRoot,
        txHash,
        status: "issued",
        createdAt: Date.now(),
      });
      setIssued({ resp: prep, txHash });
      toast({ title: "Credential issued", variant: "success" });
    } catch (err) {
      toast({ title: "Issue failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  async function showQr() {
    if (!issued) return;
    try {
      const r = await api.share(issued.resp.recordId);
      setShareUrl(r.qrUrl);
    } catch (err) {
      toast({ title: "Share failed", description: (err as Error).message, variant: "danger" });
    }
  }

  function reset() {
    setIssued(null);
    setShareUrl(null);
    setValues({});
    setErrors({});
    setDogTagId("");
  }

  if (issued) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <CheckCircle2 className="h-5 w-5 text-success" /> Credential issued
          </CardTitle>
          <CardDescription>Record {issued.resp.recordId}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-1 text-sm">
            <div>
              <span className="text-muted">Merkle root: </span>
              <span className="break-all font-mono">{issued.resp.merkleRoot}</span>
            </div>
            {issued.txHash && (
              <div>
                <span className="text-muted">Tx: </span>
                <a className="font-mono text-primary hover:underline" href={explorerTxUrl(issued.txHash)} target="_blank" rel="noreferrer">
                  {issued.txHash}
                </a>
              </div>
            )}
          </div>
          {shareUrl ? (
            <QrCode value={shareUrl} caption={shareUrl} />
          ) : (
            <Button onClick={showQr}>Show QR</Button>
          )}
          <div>
            <Button variant="ghost" onClick={reset}>
              Issue another
            </Button>
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Issue a credential</CardTitle>
        <CardDescription>
          Signing mode: <Badge variant="neutral">{signingMode}</Badge> — the document + merkle root
          are built server-side; this form supplies the credential fields (§1.6).
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="space-y-6">
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label required>Record type</Label>
              <Select value={recordType} onValueChange={setRecordType}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RECORD_TYPE_SCHEMAS.map((s) => (
                    <SelectItem key={s.recordType} value={s.recordType}>
                      {s.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label required>Dog tag id</Label>
              <Input
                value={dogTagId}
                onChange={(e) => setDogTagId(e.target.value)}
                invalid={Boolean(errors["__dogTagId"])}
                placeholder="dtag:…"
              />
              {errors["__dogTagId"] && <p className="text-xs text-danger">{errors["__dogTagId"]}</p>}
            </div>
          </div>

          {schema?.groups.map((group) => (
            <fieldset key={group.title} className="space-y-3">
              <legend className="text-sm font-semibold text-onSurface">{group.title}</legend>
              <div className="grid gap-4 sm:grid-cols-2">
                {group.fields.map((f) => (
                  <FormField
                    key={f.path}
                    def={f}
                    value={values[f.path] ?? ""}
                    error={errors[f.path]}
                    onChange={(v) => setVal(f.path, v)}
                  />
                ))}
              </div>
            </fieldset>
          ))}

          <Button type="submit" loading={busy}>
            Sign &amp; Issue
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function FormField({
  def,
  value,
  error,
  onChange,
}: {
  def: FieldDef;
  value: string;
  error?: string;
  onChange: (v: string) => void;
}) {
  return (
    <div className="space-y-1.5">
      <Label required={def.required}>{def.label}</Label>
      {def.kind === "select" ? (
        <Select value={value} onValueChange={onChange}>
          <SelectTrigger>
            <SelectValue placeholder="Choose…" />
          </SelectTrigger>
          <SelectContent>
            {def.options?.map((o) => (
              <SelectItem key={o.value} value={o.value}>
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : (
        <Input
          type={def.kind === "number" ? "number" : def.kind === "date" ? "date" : "text"}
          value={value}
          placeholder={def.placeholder}
          invalid={Boolean(error)}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
      {def.help && !error && <p className="text-xs text-muted">{def.help}</p>}
      {error && <p className="text-xs text-danger">{error}</p>}
    </div>
  );
}
