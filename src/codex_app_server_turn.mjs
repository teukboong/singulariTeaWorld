#!/usr/bin/env node

import fs from "node:fs";

const RESULT_SCHEMA_VERSION = "singulari.codex_app_server_turn_result.v1";

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
    this.nextId = 1;
    this.pending = new Map();
    this.notifications = [];
    this.agentMessages = [];
    this.serverErrors = [];
    this.serverWarnings = [];
    this.closed = false;
  }

  async connect() {
    this.ws = new WebSocket(this.url);
    this.ws.addEventListener("message", (event) => this.onMessage(event));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("websocket connect timeout")), 10_000);
      this.ws.addEventListener("open", () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
      this.ws.addEventListener("error", () => {
        clearTimeout(timer);
        reject(new Error(`websocket connection failed: ${this.url}`));
      }, { once: true });
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
    if (message.method) {
      this.notifications.push(message);
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
    }
  }

  send(method, params) {
    const id = `singulari-${this.nextId}`;
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
    this.closed = true;
    this.ws.close();
  }
}

async function waitForTurnCompleted(client, threadId, turnId, timeoutMs) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    const completed = client.notifications.find(
      (notification) =>
        notification.method === "turn/completed" &&
        notification.params?.threadId === threadId &&
        notification.params?.turn?.id === turnId,
    );
    if (completed) {
      return completed.params.turn;
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`turn completion timeout: thread=${threadId}, turn=${turnId}`);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const url = requireArg(args, "url");
  const cwd = requireArg(args, "cwd");
  const promptPath = requireArg(args, "prompt-path");
  const resultPath = requireArg(args, "result-path");
  const timeoutMs = Number(args["timeout-ms"] ?? "900000");
  const prompt = fs.readFileSync(promptPath, "utf8");
  const client = new CodexAppServerClient(url);
  let threadId = args["thread-id"] || null;
  let turnId = null;
  let failureStage = "connect";

  try {
    await client.connect();
    failureStage = "initialize";
    await client.send("initialize", {
      clientInfo: {
        name: "singulari-world-host-worker",
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

    if (threadId) {
      failureStage = "thread_resume";
      const resumed = await client.send("thread/resume", {
        threadId,
        cwd,
        excludeTurns: true,
        persistExtendedHistory: true,
      });
      threadId = resumed.thread.id;
    } else {
      failureStage = "thread_start";
      const started = await client.send("thread/start", {
        cwd,
        experimentalRawEvents: false,
        persistExtendedHistory: true,
      });
      threadId = started.thread.id;
    }

    failureStage = "turn_start";
    const startedTurn = await client.send("turn/start", {
      threadId,
      input: [
        {
          type: "text",
          text: prompt,
          text_elements: [],
        },
      ],
    });
    turnId = startedTurn.turn.id;
    failureStage = "turn_wait";
    const turn = await waitForTurnCompleted(client, threadId, turnId, timeoutMs);
    const finalMessage =
      [...client.agentMessages].reverse().find((message) => message.phase === "final_answer")
        ?.text ?? client.agentMessages.at(-1)?.text ?? "";
    writeResult(resultPath, {
      status: turn.status ?? "completed",
      thread_id: threadId,
      turn_id: turnId,
      turn_error: turn.error ?? null,
      failure_stage: null,
      final_message: finalMessage,
      agent_messages: client.agentMessages,
      server_errors: client.serverErrors,
      server_warnings: client.serverWarnings,
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
      agent_messages: client.agentMessages,
      server_errors: client.serverErrors,
      server_warnings: client.serverWarnings,
      notification_count: client.notifications.length,
    });
    try {
      client.close();
    } catch {
      // Nothing to do: this helper is already failing with the original error.
    }
    process.exitCode = 1;
  }
}

await main();
