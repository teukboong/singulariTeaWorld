# Cloudflare Free Front Door Guide

This guide gives ChatGPT web a stable HTTPS MCP URL without paying for a custom
domain. It uses a dedicated Cloudflare Worker plus Workers KV to point at the
current free `cloudflared` quick-tunnel origin.

Do not reuse Railbot files, KV namespaces, or secrets. Reuse only the pattern.

## Runtime Shape

```text
ChatGPT custom app
  -> https://<worker>.workers.dev/mcp
  -> Cloudflare Worker reads KV key "origin"
  -> https://xxxx.trycloudflare.com/mcp
  -> cloudflared quick tunnel
  -> http://127.0.0.1:4187/mcp
  -> singulari-world-mcp-web --profile play
```

The quick-tunnel hostname can rotate. The Worker URL stays stable, and
`scripts/run_mcp_tunnel.sh` updates the KV origin whenever `cloudflared` prints
a new public URL.

## One-Time Cloudflare Setup

Install the local tools:

```bash
brew install cloudflared
```

`wrangler` runs through `npx`, so Node.js must also be available.

Create a Workers KV namespace:

```bash
cd cloudflare/worker
npx wrangler kv namespace create SINGULARI_WORLD_KV
```

Copy the produced namespace id into the repository-local `.env`:

```bash
SINGULARI_WORLD_CF_KV_NAMESPACE_ID=<kv namespace id>
SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET=<random long secret>
SINGULARI_WORLD_FRONTDOOR_URL=https://<worker>.workers.dev
SINGULARI_WORLD_TUNNEL_TARGET_URL=http://127.0.0.1:4187
```

Deploy the dedicated Worker from the repository root:

```bash
scripts/deploy_cloudflare_frontdoor.sh
```

That script writes a generated Wrangler config when `wrangler.toml` still has
the placeholder KV id, deploys the Worker, and sets the Worker secret
`ORIGIN_UPDATE_SECRET` from `SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET`.

## Run the Public MCP Endpoint

Start the local Streamable HTTP MCP server:

```bash
target/release/singulari-world-mcp-web \
  --host 127.0.0.1 \
  --port 4187 \
  --path /mcp \
  --profile play
```

Start the tunnel updater in another terminal:

```bash
scripts/run_mcp_tunnel.sh
```

When the Worker accepts the origin update, the stable ChatGPT MCP URL is:

```text
https://<worker>.workers.dev/mcp
```

Use that URL in ChatGPT developer mode's custom MCP app form. The expected
warning is that this is a custom, unverified MCP server; acknowledge it only for
your own deployment.

## Local State Files

The tunnel script maintains:

```text
.runtime/mcp_tunnel_base_url.txt
.runtime/mcp_tunnel_origin_pending.txt
```

`mcp_tunnel_base_url.txt` is the last origin successfully synced into Worker
KV. `mcp_tunnel_origin_pending.txt` records a public URL that still needs a KV
retry. Both are runtime state and must stay out of git.

## Safety Boundary

- Keep `singulari-world-mcp-web` bound to `127.0.0.1`.
- Expose only `--profile play` through the public Worker.
- Do not expose `--profile trusted-local` through Cloudflare.
- Do not commit Cloudflare tokens, Worker secrets, `.env`, or `.runtime/`.
- The Worker origin update endpoint requires
  `X-Singulari-Origin-Update-Secret`.
- The Worker accepts HTTPS origins only and defaults to tunnel host suffixes
  such as `*.trycloudflare.com`, `*.ts.net`, and `*.loca.lt`.

## Troubleshooting

- `cloudflared not found`: install it with `brew install cloudflared`.
- Worker deploy asks for auth: run the browser login prompted by Wrangler, then
  rerun `scripts/deploy_cloudflare_frontdoor.sh`.
- ChatGPT sees 502/503: check that `singulari-world-mcp-web` is listening on
  `127.0.0.1:4187` and that `scripts/run_mcp_tunnel.sh` is still running.
- Tunnel URL changed but Worker still points to the old one: check
  `.runtime/mcp_tunnel_origin_pending.txt`, the front-door URL, and
  `SINGULARI_WORLD_FRONTDOOR_UPDATE_SECRET`.
- Custom app cannot connect: paste the Worker `/mcp` URL, not the temporary
  `trycloudflare.com` URL, unless you are doing a one-off debug.
