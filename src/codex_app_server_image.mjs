#!/usr/bin/env node

import fs from "node:fs";

const RESULT_SCHEMA_VERSION = "singulari.codex_app_server_image_result.v1";

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith("--")) {
      throw new Error(`unexpected positional argument: ${key}`);
    }
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`missing value for ${key}`);
    }
    args[key.slice(2)] = value;
    index += 1;
  }
  return args;
}

function requireArg(args, name) {
  const value = args[name];
  if (!value || value.trim() === "") {
    throw new Error(`missing --${name}`);
  }
  return value;
}

function writeResult(path, payload) {
  fs.writeFileSync(
    path,
    `${JSON.stringify(
      {
        schema_version: RESULT_SCHEMA_VERSION,
        generated_at: new Date().toISOString(),
        ...payload,
      },
      null,
      2,
    )}\n`,
  );
}

class CodexAppServerClient {
  constructor(url) {
    this.url = url;
    this.startedAt = Date.now();
    this.nextId = 1;
    this.pending = new Map();
    this.notifications = [];
    this.timeline = [];
    this.imageGenerations = [];
    this.agentMessages = [];
    this.serverErrors = [];
    this.serverWarnings = [];
  }

  mark(event, fields = {}) {
    this.timeline.push({
      event,
      elapsed_ms: Date.now() - this.startedAt,
      ...fields,
    });
  }

  async connect() {
    this.ws = new WebSocket(this.url);
    this.ws.addEventListener("message", (event) => this.onMessage(event));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("websocket connect timeout")), 10_000);
      this.ws.addEventListener(
        "open",
        () => {
          clearTimeout(timer);
          this.mark("websocket_open");
          resolve();
        },
        { once: true },
      );
      this.ws.addEventListener(
        "error",
        () => {
          clearTimeout(timer);
          reject(new Error(`websocket connection failed: ${this.url}`));
        },
        { once: true },
      );
    });
  }

  onMessage(event) {
    const message = JSON.parse(String(event.data));
    if (message.id && this.pending.has(message.id)) {
      const { resolve, reject } = this.pending.get(message.id);
      this.pending.delete(message.id);
      if (message.error) {
        reject(new Error(JSON.stringify(message.error)));
      } else {
        resolve(message.result);
      }
      return;
    }
    if (!message.method) {
      return;
    }
    this.notifications.push(message);
    if (
      message.method === "turn/completed" ||
      message.method === "item/completed" ||
      message.method === "error" ||
      message.method === "warning"
    ) {
      this.mark(message.method, {
        item_type: message.params?.item?.type ?? null,
        item_status: message.params?.item?.status ?? null,
        has_saved_path: Boolean(message.params?.item?.savedPath),
      });
    }
    if (message.method === "error") {
      this.serverErrors.push(message.params ?? message);
    }
    if (message.method === "warning") {
      this.serverWarnings.push(message.params ?? message);
    }
    const item = message.params?.item;
    if (message.method === "item/completed" && item?.type === "agentMessage") {
      this.agentMessages.push({
        id: item.id ?? null,
        phase: item.phase ?? null,
        text: item.text ?? "",
      });
    }
    if (message.method === "item/completed" && item?.type === "imageGeneration") {
      const resultBytes = typeof item.result === "string" ? Buffer.byteLength(item.result) : null;
      const sanitizedRaw = { ...item };
      if (typeof sanitizedRaw.result === "string") {
        sanitizedRaw.result = `[base64:${resultBytes} bytes]`;
      }
      this.imageGenerations.push({
        id: item.id ?? null,
        status: item.status ?? null,
        result_bytes: resultBytes,
        saved_path: item.savedPath ?? null,
        revised_prompt: item.revisedPrompt ?? null,
        raw: sanitizedRaw,
      });
    }
  }

  send(method, params) {
    const id = `singulari-image-${this.nextId}`;
    this.nextId += 1;
    const request = { id, method, params };
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify(request));
    });
  }

  notify(method, params = {}) {
    this.ws.send(JSON.stringify({ method, params }));
  }

  close() {
    this.ws.close();
  }
}

async function waitForImageSavedPath(client, threadId, turnId, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const generated = [...client.imageGenerations].reverse().find((item) => item.saved_path);
    if (generated) {
      return generated;
    }
    const failedTurn = client.notifications.find(
      (notification) =>
        notification.method === "turn/completed" &&
        notification.params?.threadId === threadId &&
        notification.params?.turn?.id === turnId &&
        notification.params?.turn?.status === "failed",
    );
    if (failedTurn) {
      throw new Error(`image turn failed: thread=${threadId}, turn=${turnId}`);
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`image savedPath timeout: thread=${threadId}, turn=${turnId}`);
}

function imageHostPrompt(rawPrompt) {
  return `You are the Codex App image generation host for Singulari World.

Use the Codex App image generation capability exactly once for the prompt below.
Do not use external image providers, browser search, shell scripts, SVG placeholders, or local drawing fallbacks.
Call image generation immediately. Do not write explanatory prose.

IMAGE PROMPT:
${rawPrompt}`;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const url = requireArg(args, "url");
  const cwd = requireArg(args, "cwd");
  const promptPath = requireArg(args, "prompt-path");
  const resultPath = requireArg(args, "result-path");
  const timeoutMs = Number(args["timeout-ms"] ?? "900000");
  const rawPrompt = fs.readFileSync(promptPath, "utf8");
  const client = new CodexAppServerClient(url);
  let threadId = null;
  let turnId = null;
  let failureStage = "connect";

  try {
    await client.connect();
    failureStage = "initialize";
    await client.send("initialize", {
      clientInfo: {
        name: "singulari-world-image-host-worker",
        version: "0.1.0",
      },
      capabilities: {
        experimentalApi: true,
        optOutNotificationMethods: [
          "item/agentMessage/delta",
          "item/reasoning/textDelta",
          "item/reasoning/summaryTextDelta",
          "command/exec/outputDelta",
        ],
      },
    });
    client.notify("initialized");
    client.mark("initialized");

    failureStage = "thread_start";
    const startedThread = await client.send("thread/start", {
      cwd,
      experimentalRawEvents: false,
      persistExtendedHistory: false,
    });
    threadId = startedThread.thread.id;
    client.mark("thread_started", { thread_id: threadId });

    failureStage = "turn_start";
    const startedTurn = await client.send("turn/start", {
      threadId,
      summary: "none",
      input: [
        {
          type: "text",
          text: imageHostPrompt(rawPrompt),
          text_elements: [],
        },
      ],
    });
    turnId = startedTurn.turn.id;
    client.mark("turn_started", { thread_id: threadId, turn_id: turnId });

    failureStage = "image_wait";
    const generated = await waitForImageSavedPath(client, threadId, turnId, timeoutMs);
    if (!generated) {
      throw new Error("image generation completed without savedPath");
    }
    if (!fs.existsSync(generated.saved_path)) {
      throw new Error(`image generation savedPath does not exist: ${generated.saved_path}`);
    }
    writeResult(resultPath, {
      status: "completed",
      thread_id: threadId,
      turn_id: turnId,
      failure_stage: null,
      saved_path: generated.saved_path,
      revised_prompt: generated.revised_prompt,
      image_generations: client.imageGenerations,
      agent_messages: client.agentMessages,
      server_errors: client.serverErrors,
      server_warnings: client.serverWarnings,
      timeline: client.timeline,
      notification_count: client.notifications.length,
    });
    client.close();
  } catch (error) {
    writeResult(resultPath, {
      status: "failed",
      thread_id: threadId,
      turn_id: turnId,
      failure_stage: failureStage,
      error: error instanceof Error ? error.message : String(error),
      image_generations: client.imageGenerations,
      agent_messages: client.agentMessages,
      server_errors: client.serverErrors,
      server_warnings: client.serverWarnings,
      timeline: client.timeline,
      notification_count: client.notifications.length,
    });
    try {
      client.close();
    } catch {
      // Keep the original error as the failure reason.
    }
    process.exitCode = 1;
  }
}

await main();
