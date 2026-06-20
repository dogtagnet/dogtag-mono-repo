# TUNNELING — exposing your Mac/server to a real phone

**Goal / you'll end with:** public HTTPS URLs (via `cloudflared`) that a real phone can reach from
*any* network — cellular, corporate Wi-Fi, guest Wi-Fi — wired into the demo so the scanned QR and the
prover-service all resolve on the device.

**Audience:** an AI agent runs the fenced blocks top-to-bottom; a human follows the same steps.

This doc OWNS the canonical **tunnel map** (§3). Other docs link here instead of copying it:
[LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md), [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md),
[MOBILE_BUILD.md](./MOBILE_BUILD.md).

**Placeholders** (defined once, used throughout):

- `<LAN_IP>` = your Mac's LAN address. Get it: `ipconfig getifaddr en0`.
- `<VET_TUNNEL_URL>` = the `https://<sub>.trycloudflare.com` printed for the vet tunnel (port 41874).
- `<GROOMER_TUNNEL_URL>` = the URL printed for the groomer tunnel (port 43618).
- `<PROVER_TUNNEL_URL>` = the URL printed for the prover-service tunnel (port 41875).

---

## 1. Why you need this

A phone on a typical network **cannot reach your Mac's LAN IP**:

- **Cellular** — the phone is on the carrier's network, nowhere near your LAN.
- **Corporate / guest Wi-Fi** — these almost always enable **client (AP) isolation**, which blocks
  device-to-device traffic on the same SSID. Your phone and Mac can both see the internet but not each
  other.

A **tunnel** solves this: `cloudflared` opens an outbound connection from your Mac to Cloudflare and
hands you a **public HTTPS URL**. Anything that can reach the internet — your phone, on any network —
can reach that URL, which Cloudflare forwards back to a `localhost:<port>` on your Mac. No port
forwarding, no firewall changes, real TLS.

---

## 2. Decision: do you even need a tunnel?

Forks:

- **Phone is on the SAME Wi-Fi as the Mac AND that network has NO client isolation** (typical home
  router) → **you do NOT need a tunnel.** Use the Mac's LAN IP. Go to §2a.
- **Anything else** (cellular, corporate/guest Wi-Fi, or you're not sure about isolation) → **use
  tunnels.** Go to §3.

Not sure if your Wi-Fi isolates clients? Try §2a first; if a scan/import fails to reach the Mac, fall
back to tunnels (§3).

### 2a. Same-Wi-Fi, no isolation (LAN IP, no tunnel)

`scripts/demo-up.sh` already wires the LAN IP into the QR host via the `LAN_IP` knob. Boot with your
real LAN IP so the QR points at a host the phone can reach (the default placeholder won't match your
network).

```bash
# Get your Mac's LAN IP, then boot the demo with it as the QR host.
LAN_IP=$(ipconfig getifaddr en0) scripts/demo-up.sh
# The vet/groomer QR hosts now point at http://<LAN_IP>:41874 and :43618.
```

The Android app permits **cleartext HTTP for the demo** (`usesCleartextTraffic=true`), so a plain
`http://<LAN_IP>:<port>` host works on-device — no TLS needed on the LAN path.

**Verify.** From the phone's browser (same Wi-Fi), open `http://<LAN_IP>:41874/health` →
`{"status":"ok"}`.

**STOP if** the phone can't load that URL → the network is isolating clients (or it's not actually the
same Wi-Fi). Symptom: timeout / "can't connect". Fix: use tunnels — go to §3.

> For a 32-bit-only Android device you ALSO need a reachable prover-service. On the LAN that is
> `http://<LAN_IP>:41875`; set it as the in-app `prover_api` (see [MOBILE_BUILD.md](./MOBILE_BUILD.md)).
> The baked default points at a dead tunnel — see §7.

---

## 3. The 3 tunnels (canonical map)

The demo exposes three backends a phone may need to reach. Each gets its own `cloudflared` tunnel.

| service | local port | env var | how the phone gets the URL | in the QR? |
|---|---|---|---|---|
| vet api | `41874` | `VET_PUBLIC_URL` | `demo-up.sh` sets the vet `DEPLOYMENT_URL` → embedded as the scanned-QR host | **YES** |
| groomer api | `43618` | `GROOMER_PUBLIC_URL` | `demo-up.sh` sets the groomer `DEPLOYMENT_URL` → embedded as the scanned-QR host | **YES** |
| prover-service | `41875` | `PROVER_PUBLIC_URL` | NOT in any QR — phone reads `AppConfig.DEFAULT_PROVER_API` (baked) or the in-app `prover_api` override | **NO** |

### The asymmetry — read this

The three tunnels are NOT delivered to the phone the same way:

- **vet + groomer ride in the QR.** When you set `VET_PUBLIC_URL` / `GROOMER_PUBLIC_URL`, `demo-up.sh`
  puts that URL into the stack's `DEPLOYMENT_URL`, which becomes the **host baked into every QR code**
  the portal generates. The phone scans `/p/<token>` (vet, issue a dog tag) or `/x/<token>` (groomer,
  export/verify) and calls *only that scanned host*. So updating the tunnel + re-booting is enough —
  the phone learns the new URL automatically the next time it scans.
- **The prover URL must be set ON the phone.** The prover-service is never referenced by a QR. A
  32-bit-only Android device reads its prover URL from `AppConfig.DEFAULT_PROVER_API` (compiled in) or
  the in-app `prover_api` override. Setting `PROVER_PUBLIC_URL` only changes what URL the *prover
  process advertises*; it does **not** push anything to the phone. You must type/paste the prover URL
  into the device yourself (see [MOBILE_BUILD.md](./MOBILE_BUILD.md)).

Why does only 32-bit Android need the prover at all? 64-bit iPhones and modern arm64 Android prove the
Groth16 proof **on-device** and never call a prover. Only a 32-bit-only Android offloads proving to the
prover-service. See [MOBILE_BUILD.md](./MOBILE_BUILD.md) for the proving model.

---

## 4. Commands

Open one tunnel per service. Each `cloudflared` invocation runs in the foreground and prints a
`https://<sub>.trycloudflare.com` URL — run each in its **own terminal** (or background each) and copy
the printed URL.

```bash
# Terminal A — vet api (→ <VET_TUNNEL_URL>)
cloudflared tunnel --url http://localhost:41874

# Terminal B — groomer api (→ <GROOMER_TUNNEL_URL>)
cloudflared tunnel --url http://localhost:43618

# Terminal C — prover-service (→ <PROVER_TUNNEL_URL>)   # only if you'll use a 32-bit-only Android
cloudflared tunnel --url http://localhost:41875
```

> `cloudflared` install: `brew install cloudflared` (macOS) / `sudo apt-get install cloudflared`
> (Debian/Ubuntu). See [PREREQUISITES.md](./PREREQUISITES.md).

Then **re-boot the demo** passing the tunnel URLs (and your LAN IP, as a fallback for same-Wi-Fi
clients). This sets each stack's `DEPLOYMENT_URL` so the vet/groomer QR hosts become the public tunnel
URLs.

```bash
LAN_IP=<LAN_IP> \
VET_PUBLIC_URL=<VET_TUNNEL_URL> \
GROOMER_PUBLIC_URL=<GROOMER_TUNNEL_URL> \
PROVER_PUBLIC_URL=<PROVER_TUNNEL_URL> \
scripts/demo-up.sh
```

You only need `PROVER_PUBLIC_URL` (and the Terminal C tunnel) **if you'll use a 32-bit-only Android**.
64-bit iOS and arm64 Android never call the prover, so for those devices two tunnels
(`VET_PUBLIC_URL` + `GROOMER_PUBLIC_URL`) are enough.

After re-boot, set the phone's `prover_api` to `<PROVER_TUNNEL_URL>` (32-bit Android only) — re-booting
`demo-up.sh` does NOT push the prover URL to the phone (see §3 asymmetry, §5, §7).

**Verify.** Per tunnel, run the §6 health check before scanning anything.

---

## 5. Ephemerality — the #1 gotcha

Free `trycloudflare.com` URLs are **ephemeral**:

- they **change on EVERY run** of `cloudflared` (a fresh random subdomain each time), and
- they **drop overnight** / after idle (the tunnel dies and the URL stops resolving).

So a setup that worked yesterday will silently break. **After ANY tunnel change** (you restarted
`cloudflared`, it died overnight, or you rebooted your Mac), do BOTH of these:

1. **Re-boot `demo-up.sh`** with the new `VET_PUBLIC_URL` / `GROOMER_PUBLIC_URL` (§4). The vet/groomer
   QR host updates automatically — the phone picks it up on the next scan.
2. **Re-set the phone's `prover_api`** to the new `<PROVER_TUNNEL_URL>` (32-bit Android only). Nothing
   updates this for you — see §3 (the prover URL lives on the phone, not in the QR).

**STOP if** a scan or export/import that worked earlier suddenly fails (timeout, TLS error, "host not
found") after some time has passed → the trycloudflare URL almost certainly **rotated or expired**.
Fix: restart the affected `cloudflared` tunnel(s), then redo step 1 and/or step 2 above with the new
URL, and re-verify (§6).

---

## 6. Verify each tunnel

Every backend exposes `GET /health`. Check **each tunnel you started** before relying on it.

```bash
# Run one per tunnel you opened. Each must return {"status":"ok"}.
curl -fsS <VET_TUNNEL_URL>/health      ; echo
curl -fsS <GROOMER_TUNNEL_URL>/health  ; echo
curl -fsS <PROVER_TUNNEL_URL>/health   ; echo   # only if you started the prover tunnel
```

**Verify.** Each command prints `{"status":"ok"}`.

**STOP if** any of them does NOT return `{"status":"ok"}`:

- `curl: (6)`/`(7)` or a TLS/host error → the tunnel isn't up or the URL is stale/wrong. Restart that
  `cloudflared` (§4) and re-copy the printed URL; remember the URL changes every run (§5).
- 502 / "Bad Gateway" from Cloudflare → the tunnel is up but the local backend on that port isn't
  running. Confirm `demo-up.sh` is up and the service is listening on the mapped port (vet 41874 /
  groomer 43618 / prover 41875) — see [LOCAL_DEPLOYMENT.md](./LOCAL_DEPLOYMENT.md).

---

## 7. The stale-baked-prover trap

`AppConfig.DEFAULT_PROVER_API` (in
`apps/android/app/src/main/java/io/liberalize/dogtag/data/AppConfig.kt`) ships a **long-dead
trycloudflare URL** (`https://vertical-emails-escape-speech.trycloudflare.com`). It was a real tunnel
once; per §5 it expired long ago.

Consequences:

- **A 32-bit-only Android device** (`Build.SUPPORTED_64_BIT_ABIS.isEmpty()`) relies on a live
  prover-service to produce its proof. The baked default no longer resolves, so the user **MUST**
  override `prover_api` in-app to your current `<PROVER_TUNNEL_URL>` (or recompile the app with a live
  default). Without this, proving fails on that device. See
  [MOBILE_BUILD.md](./MOBILE_BUILD.md) for where the `prover_api` setting lives and how to set it.
- **64-bit devices ignore it entirely** — they prove on-device and never read `prover_api` or
  `DEFAULT_PROVER_API`. The dead baked URL is harmless for them.

This is the practical reason the prover URL is the **one manual phone setting** in the whole flow.

---

## 8. Production note

Tunnels are a **LOCAL/demo** convenience. In REMOTE/PRODUCTION you do **not** use `trycloudflare`:

- Each stack sits behind **Caddy**, which terminates **automatic HTTPS** (Let's Encrypt) on your real
  DNS hostname. The public base / QR host is the stack's real `DEPLOYMENT_URL` =
  `https://<DOMAIN>`, not a tunnel. See [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) §8.
- `remote-up.sh` does **not** start a prover-service. An operator with 32-bit-Android users must stand
  one up themselves, on **its own real TLS hostname** (the phone's `prover_api` then points there). In
  production it runs as the OWNER's own trusted prover (it sees the witness). See
  [REMOTE_DEPLOYMENT.md](./REMOTE_DEPLOYMENT.md) §8 and
  [PRODUCTION_DEPLOYMENT.md](./PRODUCTION_DEPLOYMENT.md).

Net: swap every `<sub>.trycloudflare.com` in this doc for a stable `https://<DOMAIN>` once you move off
the demo, and the ephemerality gotcha (§5) goes away.
