/**
 * Singulari World Worker: stable front door for a rotating local tunnel origin.
 *
 * KV key: "origin" (default)
 * Secret: ORIGIN_UPDATE_SECRET (required for /_singulari/origin)
 */

const DEFAULT_ORIGIN_KV_KEY = "origin";
const DEFAULT_ALLOWED_ORIGIN_SUFFIXES = "trycloudflare.com,ts.net,loca.lt";
const ADMIN_ORIGIN_PATH = "/_singulari/origin";
const HEALTH_PATH = "/_singulari/healthz";

function jsonResponse(value, init = {}) {
  return new Response(JSON.stringify(value), {
    ...init,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      ...(init.headers || {}),
    },
  });
}

function textResponse(value, init = {}) {
  return new Response(value, {
    ...init,
    headers: {
      "cache-control": "no-store",
      ...(init.headers || {}),
    },
  });
}

function allowedOriginSuffixes(env) {
  return String(env.ALLOWED_ORIGIN_SUFFIXES || DEFAULT_ALLOWED_ORIGIN_SUFFIXES)
    .split(",")
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean);
}

function hostMatchesSuffix(hostname, suffix) {
  return hostname === suffix || hostname.endsWith(`.${suffix}`);
}

function normalizeAllowedOrigin(originInput, env) {
  let originUrl;
  try {
    originUrl = new URL(originInput);
  } catch {
    return { error: "invalid_origin" };
  }
  if (originUrl.protocol !== "https:") {
    return { error: "origin_must_be_https" };
  }
  if (originUrl.username || originUrl.password) {
    return { error: "origin_must_not_include_credentials" };
  }
  if (originUrl.pathname !== "/" || originUrl.search || originUrl.hash) {
    return { error: "origin_must_be_scheme_and_host_only" };
  }

  const hostname = String(originUrl.hostname || "").toLowerCase();
  const allowed = allowedOriginSuffixes(env);
  if (!allowed.some((suffix) => hostMatchesSuffix(hostname, suffix))) {
    return { error: "origin_not_allowed" };
  }
  return { origin: `${originUrl.protocol}//${originUrl.host}` };
}

async function updateOrigin(request, env, key) {
  if (request.method !== "POST") {
    return textResponse("method_not_allowed", { status: 405 });
  }
  if (!env.ORIGIN_UPDATE_SECRET) {
    return textResponse("origin_update_not_configured", { status: 503 });
  }
  const got = request.headers.get("X-Singulari-Origin-Update-Secret") || "";
  if (got !== env.ORIGIN_UPDATE_SECRET) {
    return textResponse("unauthorized", { status: 401 });
  }

  let body;
  try {
    body = await request.json();
  } catch {
    return textResponse("invalid_json", { status: 400 });
  }

  const originInput = String((body && (body.origin || body.url)) || "").trim();
  if (!originInput) {
    return textResponse("missing_origin", { status: 400 });
  }

  const normalized = normalizeAllowedOrigin(originInput, env);
  if (normalized.error) {
    return textResponse(normalized.error, { status: 400 });
  }

  await env.SINGULARI_WORLD_KV.put(key, normalized.origin);
  return jsonResponse({ ok: true, origin: normalized.origin });
}

function unavailable(reason) {
  return jsonResponse({ ok: false, error: "upstream_unavailable", reason }, { status: 503 });
}

async function proxyToOrigin(request, env, key) {
  const origin = (await env.SINGULARI_WORLD_KV.get(key)) || "";
  if (!origin) {
    return unavailable("origin_not_configured");
  }

  let originUrl;
  try {
    originUrl = new URL(origin);
  } catch {
    return unavailable("origin_invalid");
  }

  const incoming = new URL(request.url);
  const target = new URL(incoming.toString());
  target.protocol = originUrl.protocol;
  target.host = originUrl.host;

  const headers = new Headers(request.headers);
  headers.delete("host");
  headers.set("X-Forwarded-Host", incoming.host);
  headers.set("X-Forwarded-Proto", incoming.protocol.replace(":", ""));
  headers.set("X-Singulari-External-Host", incoming.host);
  headers.set("X-Singulari-External-Proto", incoming.protocol.replace(":", ""));

  try {
    return await fetch(target.toString(), {
      method: request.method,
      headers,
      body: request.body,
      redirect: "manual",
    });
  } catch {
    return unavailable("fetch_failed");
  }
}

export default {
  async fetch(request, env) {
    const key = env.ORIGIN_KV_KEY || DEFAULT_ORIGIN_KV_KEY;
    const incoming = new URL(request.url);

    if (incoming.pathname === HEALTH_PATH) {
      return textResponse("ok\n", { status: 200 });
    }
    if (incoming.pathname === ADMIN_ORIGIN_PATH) {
      return updateOrigin(request, env, key);
    }
    if (incoming.pathname === "/" && request.method === "GET") {
      return jsonResponse({
        ok: true,
        service: "singulari-world-frontdoor",
        mcp: "/mcp",
        health: HEALTH_PATH,
      });
    }

    return proxyToOrigin(request, env, key);
  },
};
