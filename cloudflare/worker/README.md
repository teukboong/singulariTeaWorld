# Singulari World Cloudflare Worker Front Door

This is a separate Singulari World Worker deployment, not a shared Railbot
Worker. It gives ChatGPT web a stable HTTPS MCP URL while the local
`cloudflared` quick tunnel URL can rotate.

For the full operator setup, see
[`docs/cloudflare-free-frontdoor.md`](../../docs/cloudflare-free-frontdoor.md).

## Shape

- ChatGPT custom app URL: `https://<worker>.workers.dev/mcp`
- Worker KV key: `origin`
- Local quick tunnel: `https://xxxx.trycloudflare.com`
- Local MCP server: `http://127.0.0.1:4187/mcp`

The local tunnel script updates the Worker KV through `POST /_singulari/origin`
whenever `cloudflared` prints a new quick-tunnel URL. The Worker then proxies
all normal requests to the current origin.

## Deploy

Create a Workers KV namespace, put its id in `.env`, then deploy from the
repository root:

```bash
SINGULARI_WORLD_CF_KV_NAMESPACE_ID=<kv namespace id>
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<same secret>
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev

scripts/deploy_cloudflare_frontdoor.sh
```

Manual deploy also works. Put the KV id in `wrangler.toml`, then:

```bash
cd cloudflare/worker
npx wrangler deploy
```

Set the origin update secret:

```bash
npx wrangler secret put ORIGIN_UPDATE_SECRET
```

Then put the same secret in the repository-local `.env`:

```bash
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<same secret>
```

Run the local MCP server and tunnel:

```bash
cargo run --locked --bin singulari-world-mcp-web -- \
  --host 127.0.0.1 \
  --port 4187 \
  --path /mcp \
  --profile play

scripts/run_mcp_tunnel.sh
```

The script keeps `.runtime/mcp_tunnel_base_url.txt` in sync with the current
quick-tunnel URL after the Worker accepts the update.

The ChatGPT connector should use the Worker URL, not the temporary tunnel URL:

```text
https://<worker>.workers.dev/mcp
```

## Safety

`/_singulari/origin` accepts HTTPS origins only, rejects credentials, and by
default allows only known free tunnel host suffixes:

- `*.trycloudflare.com`
- `*.ts.net`
- `*.loca.lt`

Override with `ALLOWED_ORIGIN_SUFFIXES` only if the front door should proxy a
different tunnel provider.
