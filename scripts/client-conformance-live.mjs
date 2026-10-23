#!/usr/bin/env node
// Explicitly opt in. Configs, working directory and gateway data are all disposable.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import {
  Fixture,
  profiles,
  gateway,
  repo,
  stop,
  stopTree,
  wireMetadata,
  waitExit,
} from "./client-conformance-support.mjs";

const self = fileURLToPath(import.meta.url);
const listChanged = "notifications/tools/list_changed";

// Disposable client config. `mcp` is the stdio wiretap entry in the common
// `{command, args}` shape; each writer reshapes it the way the client reads it.
const json = (file, root) => (mcp) => [file, root(mcp)];
const servers = (file, extra = {}) =>
  json(file, (mcp) => ({ ...extra, mcpServers: { toolport: mcp } }));
const toml = (file) => (mcp) => [
  file,
  `[mcp_servers.toolport]\ncommand = ${JSON.stringify(mcp.command)}\nargs = ${JSON.stringify(mcp.args)}\n`,
];
const stdioTyped = (file, key) =>
  json(file, (mcp) => ({ [key]: { toolport: { type: "stdio", ...mcp } } }));

const mockCrushModel = {
  id: "mock-model",
  name: "Mock",
  context_window: 128_000,
  default_max_tokens: 1_024,
};

// `args` must not start a model turn against a real service. `driver` names a
// stdin protocol. `acp` stops after session/new unless `prompt` is set; then one
// prompt goes to the loopback mock model, for clients that list tools lazily.
const clients = {
  "claude-code": { args: ["mcp", "list"], config: servers(".claude/.claude.json") },
  cursor: {
    args: ["mcp", "list-tools", "toolport"],
    config: servers(".cursor/mcp.json"),
  },
  opencode: {
    args: ["mcp", "list"],
    config: json(".config/opencode/opencode.json", (mcp) => ({
      mcp: {
        toolport: { type: "local", command: [mcp.command, ...mcp.args], enabled: true },
      },
    })),
  },
  "kilo-code": {
    args: ["mcp", "list"],
    config: json(".config/kilo/kilo.jsonc", (mcp) => ({
      mcp: {
        toolport: { type: "local", command: [mcp.command, ...mcp.args], enabled: true },
      },
    })),
  },
  "gemini-cli": {
    args: ["mcp", "list"],
    config: servers(".gemini/settings.json", {
      security: { folderTrust: { enabled: false } },
    }),
  },
  "qwen-code": { args: ["mcp", "list"], config: servers(".qwen/settings.json") },
  // BYOK against the loopback mock; offline mode skips GitHub auth and services.
  "github-copilot-cli": {
    args: ["-p", "ping"],
    model: true,
    env: (model) => ({
      COPILOT_OFFLINE: "true",
      COPILOT_PROVIDER_BASE_URL: `${model.url}/v1`,
      COPILOT_PROVIDER_API_KEY: "mock-key",
      COPILOT_MODEL: "mock-model",
    }),
    config: json(".copilot/mcp-config.json", (mcp) => ({
      mcpServers: { toolport: { ...mcp, tools: ["*"] } },
    })),
  },
  codex: { args: ["app-server"], driver: "codex", config: toml(".codex/config.toml") },
  hermes: {
    args: ["mcp", "test", "toolport"],
    config: json(".hermes/config.yaml", (mcp) => ({ mcp_servers: { toolport: mcp } })),
  },
  droid: { args: ["mcp", "list"], config: stdioTyped(".factory/mcp.json", "mcpServers") },
  amp: {
    args: ["tools", "list"],
    config: json(".config/amp/settings.json", (mcp) => ({
      "amp.mcpServers": { toolport: mcp },
    })),
  },
  auggie: { args: ["mcp", "list"], config: servers(".augment/settings.json") },
  grok: { args: ["mcp", "doctor"], config: toml(".grok/config.toml") },
  "command-code": {
    args: ["acp"],
    driver: "acp",
    env: () => ({ CMD_LOCAL_ONLY: "1" }),
    config: servers(".commandcode/mcp.json"),
  },
  "kimi-code": {
    args: ["acp"],
    driver: "acp",
    model: true,
    prompt: true,
    config: (mcp, model) => [
      [".kimi-code/mcp.json", { mcpServers: { toolport: mcp } }],
      [
        ".kimi-code/config.toml",
        [
          'default_model = "mock/mock-model"',
          "[providers.mock]",
          `base_url = ${JSON.stringify(`${model.url}/v1`)}`,
          'type = "openai"',
          'api_key = "mock-key"',
          '[models."mock/mock-model"]',
          'provider = "mock"',
          'model = "mock-model"',
          "max_context_size = 128000",
          'capabilities = ["tool_use"]',
          "",
        ].join("\n"),
      ],
    ],
  },
  goose: {
    args: ["acp"],
    driver: "acp",
    model: true,
    prompt: true,
    env: () => ({ OPENAI_API_KEY: "mock-key", GOOSE_DISABLE_KEYRING: "1" }),
    // JSON is valid YAML. GOOSE_PATH_ROOT points the config under .goose.
    config: (mcp, model) => [
      ".goose/config/config.yaml",
      {
        GOOSE_PROVIDER: "openai",
        GOOSE_MODEL: "mock-model",
        OPENAI_HOST: model.url,
        OPENAI_BASE_PATH: "v1/chat/completions",
        extensions: {
          toolport: {
            enabled: true,
            type: "stdio",
            name: "toolport",
            cmd: mcp.command,
            args: mcp.args,
            envs: {},
            timeout: 300,
          },
        },
      },
    ],
  },
  omp: { args: ["acp"], driver: "acp", config: servers(".omp/agent/mcp.json") },
  "omp-rpc": {
    profile: "omp",
    args: ["--mode", "rpc", "--no-session"],
    driver: "hold",
    config: servers(".omp/agent/mcp.json"),
  },
  // `mcp get` and `doctor` read config only; ACP session/new connects.
  "devin-cli": {
    args: ["acp"],
    driver: "acp",
    config: servers(".config/devin/mcp_config.json"),
  },
  kiro: {
    args: ["mcp", "status", "--name", "toolport"],
    config: servers(".kiro/settings/mcp.json"),
  },
  // The same agent given the server by the ACP client in session/new.
  "omp-acp-servers": {
    profile: "omp",
    args: ["acp"],
    driver: "acp",
    acpServers: true,
    config: servers(".omp/agent/unused.json"),
  },
  // Pi has no built-in MCP. Its MCP config is read by the pi-mcp-adapter
  // extension, loaded from an existing local package path; nothing is installed.
  pi: {
    args: ["--mode", "rpc", "--no-session"],
    driver: "hold",
    config: (mcp) => [
      [".pi/agent/mcp.json", { mcpServers: { toolport: mcp } }],
      ...(process.env.TOOLPORT_LIVE_PI_PACKAGE
        ? [
            [
              ".pi/agent/settings.json",
              { packages: [process.env.TOOLPORT_LIVE_PI_PACKAGE] },
            ],
          ]
        : []),
    ],
  },
  junie: {
    args: (_home, model) => [
      "--provider=litellm",
      `--litellm-url=${model.url}`,
      "--litellm-api-key=mock-key",
      "--model=gpt-5.4",
      "ping",
    ],
    model: true,
    config: servers(".junie/mcp/mcp.json"),
  },
  continue: {
    args: (home) => ["--config", path.join(home, ".continue/config.yaml"), "-p", "ping"],
    model: true,
    config: (mcp, model) => [
      ".continue/config.yaml",
      {
        name: "toolport-conformance",
        version: "0.0.1",
        schema: "v1",
        models: [
          {
            name: "mock",
            provider: "openai",
            model: "mock-model",
            apiBase: `${model.url}/v1`,
            apiKey: "mock-key",
            roles: ["chat"],
          },
        ],
        mcpServers: [{ name: "toolport", ...mcp }],
      },
    ],
  },
  // These start MCP only with a provider configured. The provider is the
  // loopback mock model below, never a real model service.
  crush: {
    args: ["run", "ping"],
    model: true,
    config: (mcp, model) => [
      ".config/crush/crush.json",
      {
        options: { disable_provider_auto_update: true, disable_metrics: true },
        providers: {
          mock: {
            name: "Mock",
            type: "openai-compat",
            base_url: `${model.url}/v1`,
            api_key: "mock-key",
            models: [mockCrushModel],
          },
        },
        models: {
          large: { model: "mock-model", provider: "mock" },
          small: { model: "mock-model", provider: "mock" },
        },
        mcp: { toolport: { type: "stdio", ...mcp } },
      },
    ],
  },
};

if (process.argv[2] === "--wiretap") {
  // Clients may scrub the environment, so the parent passes the gateway path.
  const [trace, data, id, home, bin, probe] = process.argv.slice(3);
  const fixture = new Fixture(home, id);
  assert.equal(data, fixture.data, "wiretap fixture data mismatch");
  await appendFile(path.join(fixture.data, "wiretap.pid"), `${process.pid}\n`);
  const child = spawn(bin, ["--stdio-adapter"], {
    cwd: repo,
    env: fixture.env(),
    stdio: ["pipe", "pipe", "pipe"],
  });
  let writes = Promise.resolve();
  function record(direction, line) {
    // Retain metadata only; non-JSON chatter must not crash tracing or leak text.
    const safe = wireMetadata(line);
    writes = writes.then(() =>
      appendFile(
        trace,
        JSON.stringify({ direction, elapsedMs: performance.now(), message: safe }) + "\n",
      ),
    );
  }
  // Opt-in probe: after the first tools/list reply, tell the client its tool
  // list changed, as Toolport does when a downstream catalog changes.
  const listIds = new Set();
  let injected = probe !== "probe-list-changed";
  const input = createInterface({ input: process.stdin });
  input.on("line", (line) => {
    record("client", line);
    try {
      const message = JSON.parse(line);
      if (message?.method === "tools/list") listIds.add(JSON.stringify(message.id));
    } catch {
      /* Forwarded unchanged. */
    }
    child.stdin.write(line + "\n");
  });
  input.on("close", () => child.stdin.end());
  child.stdin.on("error", () => input.close());
  child.stderr.pipe(process.stderr);
  child.stderr.on("data", (d) => {
    writes = writes.then(() =>
      appendFile(path.join(path.dirname(trace), "gateway-stderr.log"), d),
    );
  });
  const output = createInterface({ input: child.stdout });
  output.on("line", (line) => {
    record("server", line);
    process.stdout.write(line + "\n");
    if (injected) return;
    try {
      const message = JSON.parse(line);
      if (message?.result && listIds.has(JSON.stringify(message.id))) {
        injected = true;
        setTimeout(() => {
          const note = JSON.stringify({ jsonrpc: "2.0", method: listChanged });
          record("injected", note);
          process.stdout.write(note + "\n");
        }, 500);
      }
    } catch {
      /* Not a reply. */
    }
  });
  child.on("error", (e) => {
    console.error(e.message);
    writes = writes.then(() =>
      appendFile(path.join(path.dirname(trace), "gateway-stderr.log"), e.message + "\n"),
    );
    process.exitCode = 1;
    input.close();
  });
  child.on("exit", (code) => {
    process.exitCode = code ?? 1;
    input.close();
  });
  await waitExit(child, 90_000).catch(async () => {
    await stop(child);
    process.exitCode = 1;
  });
  await writes;
} else {
  const flags = process.argv.slice(2).filter((a) => a.startsWith("--"));
  const [id, executable, output] = process.argv
    .slice(2)
    .filter((a) => !a.startsWith("--"));
  const probe = flags.includes("--probe-list-changed");
  // Default to Full so proxied tools are listed; --auto keeps the client's Auto mode.
  const auto = flags.includes("--auto");
  const spec = clients[id];
  assert(
    spec && executable && output,
    "Usage: node scripts/client-conformance-live.mjs [--probe-list-changed] [--auto] <client-id> <CLI executable> <new evidence directory>",
  );
  assert(path.isAbsolute(executable), "CLI executable must be an absolute path");
  const profile = profiles.find((p) => p.id === (spec?.profile ?? id));
  assert(profile);
  await mkdir(output, { recursive: false });
  const fixture = await Fixture.create(profile.id, { full: !auto });
  const model = spec.model
    ? await mockModel(path.join(output, "model.jsonl"))
    : undefined;
  let child;
  try {
    const trace = path.resolve(output, "traffic.jsonl");
    const shim = [
      self,
      "--wiretap",
      trace,
      fixture.data,
      profile.id,
      fixture.home,
      gateway,
    ];
    if (probe) shim.push("probe-list-changed");
    const command = process.execPath;
    const files = spec.config({ command, args: shim }, model);
    for (const [name, value] of Array.isArray(files[0]) ? files : [files]) {
      const filename = path.join(fixture.home, name);
      await mkdir(path.dirname(filename), { recursive: true });
      await writeFile(
        filename,
        typeof value === "string" ? value : JSON.stringify(value),
      );
    }
    const env = { ...fixture.env(), ...spec.env?.(model) };
    const version = await run(executable, ["--version"], fixture.home, env);
    const args =
      typeof spec.args === "function" ? spec.args(fixture.home, model) : spec.args;
    child = spawn(executable, args, {
      cwd: fixture.home,
      detached: process.platform !== "win32",
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stdin.on("error", () => {});
    let status = "",
      stderr = "";
    child.stdout.on("data", (d) => {
      status = (status + d).slice(-16_384);
    });
    child.stderr.on("data", (d) => {
      stderr = (stderr + d).slice(-16_384);
    });
    const driven = drive(spec, child, fixture.home, {
      name: "toolport",
      command,
      args: shim,
      env: [],
    });
    let timedOut = false,
      cliError;
    try {
      await Promise.race([waitExit(child, 45_000), driven.done]);
    } catch (error) {
      timedOut = error.message === "child exit deadline";
      cliError = error.message;
    }
    // Close surviving wiretaps before reading their final trace.
    await stopTree(child);
    let records = [];
    try {
      records = (await readFile(trace, "utf8"))
        .trim()
        .split("\n")
        .filter(Boolean)
        .map(JSON.parse);
    } catch (e) {
      if (e.code !== "ENOENT") throw e;
    }
    const sent = records.filter((r) => r.direction === "client");
    const initialize = sent.find((r) => r.message.method === "initialize");
    const discover = sent.find((r) => r.message.method === "server/discover");
    const notified = records.find(
      (r) => r.direction !== "client" && r.message.method === listChanged,
    );
    const summary = {
      client: id,
      clientVersion: version,
      command: args,
      discovery: auto ? "auto" : "full",
      modelRequests: model ? await model.requests() : undefined,
      driver: spec.driver ?? "exit",
      exitCode: child.exitCode,
      stoppedByDriver: driven.stopped(),
      timedOut,
      cliError,
      handshakeCaptured: Boolean(initialize || discover),
      handshake: initialize ? "initialize" : discover ? "server/discover" : null,
      initialize: initialize?.message,
      discover: discover?.message,
      subscriptions: sent
        .filter((r) => r.message.method === "subscriptions/listen")
        .map((r) => r.message.params),
      methods: sent.map((r) => r.message.method).filter(Boolean),
      serverRequests: records
        .filter(
          (r) => r.direction === "server" && r.message.method && r.message.id != null,
        )
        .map((r) => r.message.method),
      errorCodes: records
        .filter((r) => r.message.errorCode != null)
        .map((r) => ({
          direction: r.direction,
          id: r.message.id,
          code: r.message.errorCode,
        })),
      listChangedProbe: notified
        ? {
            source: notified.direction,
            relisted: sent.some(
              (r) =>
                r.elapsedMs > notified.elapsedMs && r.message.method === "tools/list",
            ),
          }
        : probe
          ? { source: null, relisted: null }
          : undefined,
      scope:
        "isolated non-model command only; no authenticated model call or GUI acceptance",
    };
    // No auth was supplied and only fixture status is retained.
    await writeFile(path.join(output, "status.txt"), status + stderr);
    await writeFile(
      path.join(output, "summary.json"),
      JSON.stringify(summary, null, 2) + "\n",
    );
    console.log(JSON.stringify(summary));
    const exited = driven.stopped() || (!timedOut && child.exitCode === 0);
    if (!(initialize || discover) || !exited) process.exitCode = 1;
  } finally {
    if (child) await stopTree(child);
    model?.close();
    await fixture.close();
  }
}

// Bounded `--version` in the same disposable environment.
async function run(executable, args, cwd, env) {
  const child = spawn(executable, args, {
    cwd,
    detached: process.platform !== "win32",
    env,
    stdio: ["ignore", "pipe", "pipe"],
  });
  let text = "";
  child.stdout.on("data", (d) => (text = (text + d).slice(0, 4_096)));
  child.stderr.on("data", (d) => (text = (text + d).slice(0, 4_096)));
  try {
    await waitExit(child, 20_000);
  } catch {
    /* Recorded as unknown below. */
  }
  await stopTree(child);
  // Some CLIs print warnings first; prefer the line that carries a version.
  const lines = text
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  return lines.find((l) => /\d+\.\d+/.test(l)) ?? lines[0] ?? null;
}

// Speak just enough of a stdin protocol to start MCP without a prompt.
function drive({ driver: kind, prompt, acpServers }, child, cwd, server) {
  let stopped = false;
  let finish;
  const done = new Promise((resolve) => (finish = resolve));
  const settle = (ms) =>
    setTimeout(() => {
      stopped = true;
      finish();
    }, ms);
  const send = (message) => child.stdin.write(JSON.stringify(message) + "\n");
  if (kind === "codex") {
    // Ask the app server for MCP status without starting a model turn.
    const responses = createInterface({ input: child.stdout });
    let requested = false;
    responses.on("line", (line) => {
      try {
        const reply = JSON.parse(line);
        if (reply.id === 1 && !requested) {
          requested = true;
          send({ method: "initialized", params: {} });
          send({ id: 2, method: "mcpServerStatus/list", params: {} });
        }
        if (reply.id === 2) child.stdin.end();
      } catch {
        /* Human CLI output is retained in status, not wire evidence. */
      }
    });
    send({
      id: 1,
      method: "initialize",
      params: {
        clientInfo: { name: "toolport-conformance", version: "1" },
        capabilities: { experimentalApi: true },
      },
    });
  } else if (kind === "acp") {
    // ACP initialize, then session/new. The session's MCP servers come from the
    // disposable config file. No session/prompt is ever sent.
    const responses = createInterface({ input: child.stdout });
    responses.on("line", (line) => {
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        return;
      }
      if (message.method && message.id != null)
        send({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32601, message: "not supported by conformance driver" },
        });
      else if (message.id === 1)
        send({
          jsonrpc: "2.0",
          id: 2,
          method: "session/new",
          params: { cwd, mcpServers: acpServers ? [server] : [] },
        });
      else if (message.id === 2 && prompt && message.result?.sessionId)
        send({
          jsonrpc: "2.0",
          id: 3,
          method: "session/prompt",
          params: {
            sessionId: message.result.sessionId,
            prompt: [{ type: "text", text: "ping" }],
          },
        });
      else if (message.id === 2 || message.id === 3) settle(12_000);
    });
    send({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: 1,
        clientCapabilities: {
          fs: { readTextFile: false, writeTextFile: false },
          terminal: false,
        },
        clientInfo: { name: "toolport-conformance", version: "1" },
      },
    });
  } else if (kind === "hold") {
    // Long-lived modes that load MCP at startup; keep stdin open, then stop.
    settle(20_000);
  } else child.stdin.end();
  return { done, stopped: () => stopped };
}

// Loopback OpenAI-compatible model. It answers "ok" and records only the
// request path and the tool names a client offered the model.
async function mockModel(log) {
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body = (body + d).slice(0, 4_000_000)));
    req.on("end", async () => {
      let request = {};
      try {
        request = JSON.parse(body || "{}");
      } catch {
        /* Path only. */
      }
      const tools = (request.tools || []).map((t) => t.function?.name ?? t.name);
      await appendFile(
        log,
        JSON.stringify({
          method: req.method,
          path: req.url,
          stream: request.stream,
          tools,
        }) + "\n",
      );
      const created = Math.floor(Date.now() / 1000);
      if (req.method === "GET" && req.url.endsWith("/models")) {
        res.setHeader("content-type", "application/json");
        return res.end(
          JSON.stringify({
            object: "list",
            data: [{ id: "mock-model", object: "model", owned_by: "toolport", created }],
          }),
        );
      }
      if (!req.url.endsWith("/chat/completions")) {
        res.statusCode = 404;
        return res.end();
      }
      const base = { id: "mock", created, model: "mock-model" };
      const usage = { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 };
      if (!request.stream) {
        res.setHeader("content-type", "application/json");
        return res.end(
          JSON.stringify({
            ...base,
            object: "chat.completion",
            choices: [
              {
                index: 0,
                message: { role: "assistant", content: "ok" },
                finish_reason: "stop",
              },
            ],
            usage,
          }),
        );
      }
      res.setHeader("content-type", "text/event-stream");
      const chunk = (choice, extra = {}) =>
        res.write(
          `data: ${JSON.stringify({ ...base, object: "chat.completion.chunk", choices: [{ index: 0, ...choice }], ...extra })}\n\n`,
        );
      chunk({ delta: { role: "assistant", content: "ok" }, finish_reason: null });
      chunk({ delta: {}, finish_reason: "stop" }, { usage });
      res.end("data: [DONE]\n\n");
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  return {
    url: `http://127.0.0.1:${server.address().port}`,
    async requests() {
      try {
        return (await readFile(log, "utf8"))
          .trim()
          .split("\n")
          .filter(Boolean)
          .map(JSON.parse);
      } catch {
        return [];
      }
    },
    close: () => server.close(),
  };
}
