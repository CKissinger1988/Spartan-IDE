# Deploying Spartan services and pairing mobile clients

Spartan has two server forms with different trust boundaries:

| Service | Intended use | Default bind | Mobile QR contents |
|---|---|---|---|
| `spartan-devserver` | A private Linux workstation or development host | `127.0.0.1:4400` | Endpoint and private pairing secret |
| `spartan-cloud-api` | The optional multi-tenant cloud control plane | `127.0.0.1:8080` | HTTPS endpoint only |

Neither binary silently updates itself. `--check-update` only compares the installed version with
the latest Spartan GitHub Release and prints the official release URL.

## Private Linux server

Build the web client and the server, then run it from the project you intend to expose:

```bash
cargo build --release -p spartan-devserver
(cd web && npm ci && npm run build)
cd /path/to/project
/path/to/Spartan-IDE/target/release/spartan-devserver
```

The default loopback bind is appropriate for one machine. For a trusted LAN or an explicitly
secured WAN deployment, choose one concrete address and supply a long random pairing secret:

```bash
spartan-devserver \
  --host:192.168.1.20 \
  --mobile-pairing-token:replace-with-a-long-random-secret \
  --print-mobile-qr
```

The devserver rejects wildcard addresses (`0.0.0.0` and `::`) and rejects every non-loopback bind
without a pairing secret. A non-loopback mobile handoff must present that secret. This reduces
accidental exposure but does not add transport encryption: use HTTPS/WSS at a reverse proxy before
using a public network. Treat a private pairing QR as a credential and rotate the secret if the QR
or paired device is lost.

The paired endpoint may be pasted or scanned in Spartan Mobile's first-run or Settings flow. The
mobile app keeps the private pairing secret in platform secure storage; it is not placed in normal
app storage or a URL after import.

## SSH-only access

The least exposed remote arrangement keeps the private devserver loopback-only on the remote host
and forwards it over SSH:

```bash
scripts/spartan-ssh-forward --host dev.example.com --user spartan
```

The helper exposes `127.0.0.1:4400` locally, enables SSH keepalives, and uses
`ExitOnForwardFailure` so it does not claim success without a tunnel. Optional `--remote-port`,
`--local-port`, and `--identity` values support non-default ports and a chosen key. Pair the client
with the forwarded endpoint; do not open the private RPC port directly to the WAN in this mode.

## Spartan Cloud

Place the cloud API behind a TLS reverse proxy and use the public HTTPS origin in the QR:

```bash
SPARTAN_CLOUD_VAULT_KEY=64-hex-character-master-key \
SPARTAN_CLOUD_ADMIN_EMAIL=admin@example.com \
SPARTAN_CLOUD_ADMIN_PASSWORD=choose-a-strong-password \
spartan-cloud-api \
  --bind:0.0.0.0:8080 \
  --rp-origin:https://cloud.example.com \
  --public-origin:https://cloud.example.com \
  --print-mobile-qr
```

`GET /api/health` is deliberately public for load balancers and connection diagnostics. Tenant API
operations still require their existing bearer-session authentication. A cloud pairing QR carries
only the HTTPS endpoint; it never carries a bearer token, password, allocation capability, or
secrets-vault key. Mobile users authenticate normally after pairing.

The `--rp-origin` host must be the real WebAuthn relying-party domain, not a bare IP address. The
vault stays locked unless `SPARTAN_CLOUD_VAULT_KEY` is exactly 64 hexadecimal characters. Review
[cloud/README.md](../cloud/README.md) before enabling container allocation: the API deliberately
refuses tenant allocation until the operator has independently verified its OCI isolation setup.

## Update checks and release verification

```bash
spartan-devserver --check-update
spartan-cloud-api --check-update
```

Release artifacts include SHA-256 checksums. Verify a downloaded server binary or package against
the checksum published on the corresponding GitHub Release before installing it. Desktop uses its
native updater flow; Android only presents an explicit GitHub-owned download action because an app
cannot silently replace its installed APK.
