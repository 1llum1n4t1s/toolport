#!/usr/bin/env node
// Opt-in, Linux-only black-box comparison. Uses only Node built-ins.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import { cpus, release, tmpdir } from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";

const here = path.dirname(fileURLToPath(import.meta.url));
const fixtureDir = path.resolve(here, "../src-tauri/tests/fixtures/search-eval-scale");
const mock = path.join(here, "gateway-compare-mock.mjs");
const call = { name: "slack__slack_list_channels", arguments: { limit: 5 } };
const active = new Set();
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = (value) => Buffer.byteLength(JSON.stringify(value));

export function summary(values) {
  assert(values.length > 0, "empty measurement");
  const sorted = [...values].sort((a, b) => a - b);
  const percentile = (p) => sorted[Math.ceil(p * sorted.length) - 1];
  return {
    n: values.length,
    p50: percentile(0.5),
    p95: percentile(0.95),
    min: sorted[0],
    max: sorted.at(-1),
  };
}

function environment(home) {
  const data = path.join(home, "gateway-data");
  return {
    PATH: process.env.PATH,
    HOME: home,
    TMPDIR: home,
    XDG_CONFIG_HOME: path.join(home, "config"),
    XDG_DATA_HOME: path.join(home, "data"),
    XDG_CACHE_HOME: path.join(home, "cache"),
    XDG_RUNTIME_DIR: path.join(home, "run"),
    DBUS_SESSION_BUS_ADDRESS: `unix:path=${home}/no-session-bus`,
    TOOLPORT_DATA_DIR: data,
    TOOLPORT_REGISTRY: path.join(data, "registry.json"),
    TOOLPORT_CODE_MODE: "0",
    TOOLPORT_SECRET_KEY: "disposable-public-fixture-key",
  };
}

class Session {
  constructor(binary, env, home) {
    this.started = performance.now();
    this.pending = new Map();
    this.nextId = 1;
    this.stderr = "";
    this.proc = spawn(binary, ["--stdio-adapter"], { env, cwd: home, stdio: "pipe" });
    this.proc.stderr.on("data", (data) => {
      this.stderr = (this.stderr + data).slice(-2000);
    });
    this.proc.stdin.on("error", (error) => this.fail(error));
    this.proc.on("error", (error) => this.fail(error));
    this.proc.on("exit", (code, signal) =>
      this.fail(new Error(`adapter exited: ${code ?? signal}; ${this.stderr}`)),
    );
    this.lines = createInterface({ input: this.proc.stdout });
    this.lines.on("line", (line) => {
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        return;
      }
      if (message.method && message.id !== undefined) {
        this.send({
          jsonrpc: "2.0",
          id: message.id,
          result: message.method === "roots/list" ? { roots: [] } : {},
        });
      } else if (!message.method) {
        const pending = this.pending.get(message.id);
        if (pending) {
          this.pending.delete(message.id);
          clearTimeout(pending.timer);
          if (message.error || message.result?.isError) {
            pending.reject(new Error(`RPC failed: ${JSON.stringify(message)}`));
          } else {
            pending.resolve({
              result: message.result,
              ms: performance.now() - pending.started,
            });
          }
        }
      }
    });
  }
  fail(error) {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }
  send(message) {
    this.proc.stdin.write(JSON.stringify(message) + "\n");
  }
  request(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out; ${this.stderr}`));
      }, 60_000);
      this.pending.set(id, { resolve, reject, timer, started: performance.now() });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }
  async initialize() {
    await this.request("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "bench", version: "bench" },
    });
    this.send({ jsonrpc: "2.0", method: "notifications/initialized" });
  }
  async close() {
    this.fail(new Error("session closed"));
    this.proc.stdin.end();
    if (this.proc.exitCode !== null || this.proc.signalCode !== null) return;
    await Promise.race([
      new Promise((resolve) => this.proc.once("exit", resolve)),
      sleep(1000),
    ]);
    if (this.proc.exitCode === null && this.proc.signalCode === null) {
      this.proc.kill("SIGKILL");
      await new Promise((resolve) => this.proc.once("exit", resolve));
    }
    this.lines.close();
  }
}

// Own only the exact temp HOME or mock arguments generated for this fixture.
// Read /proc privately; never print command lines or inherited environments.
async function owns(pid, home) {
  try {
    const env = (await readFile(`/proc/${pid}/environ`, "utf8")).split("\0");
    const args = (await readFile(`/proc/${pid}/cmdline`, "utf8")).split("\0");
    return (
      env.includes(`HOME=${home}`) ||
      (args.includes(mock) &&
        args.includes(path.join(home, "catalog.json")) &&
        args.includes(path.join(home, "calls.jsonl")))
    );
  } catch {
    return false;
  }
}
function signal(pid, name) {
  try {
    process.kill(pid, name);
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}
async function ownedPids(home) {
  const pids = (await readdir("/proc")).filter(
    (p) => /^\d+$/.test(p) && Number(p) !== process.pid,
  );
  const owned = await Promise.all(
    pids.map(async (pid) => ((await owns(pid, home)) ? Number(pid) : null)),
  );
  return owned.filter((pid) => pid !== null);
}

class Fixture {
  static async create(binary, major, catalog) {
    const fx = new Fixture();
    fx.home = await mkdtemp(path.join(tmpdir(), "toolport-compare-"));
    fx.sessions = [];
    active.add(fx);
    try {
      fx.binary = binary;
      fx.env = environment(fx.home);
      fx.data = fx.env.TOOLPORT_DATA_DIR;
      fx.trace = path.join(fx.home, "calls.jsonl");
      await mkdir(fx.data, { mode: 0o700 });
      await mkdir(fx.env.XDG_RUNTIME_DIR, { mode: 0o700 });
      const catalogPath = path.join(fx.home, "catalog.json");
      await writeFile(catalogPath, JSON.stringify(catalog));
      const namespaces = [...new Set(catalog.map((t) => t.name.split("__")[0]))];
      const registry = {
        version: major < 2 ? 1 : 3,
        servers: namespaces.map((ns) => ({
          id: ns,
          name: ns,
          transport: "stdio",
          command: process.execPath,
          args: [mock, catalogPath, ns, fx.trace],
          env: [],
          ...(major >= 2 ? { enabled: true, inheritEnv: false } : {}),
        })),
        profiles: [],
        clientDiscovery: {
          "bench-perf": "full",
          "client:bench-perf": "full",
          "bench-search": "lazy",
          "client:bench-search": "lazy",
        },
      };
      if (major >= 2) registry.safetyLevel = "off";
      else {
        registry.profiles = [
          { id: "default", name: "Default", enabledServerIds: namespaces },
        ];
        registry.activeProfileId = "default";
      }
      await writeFile(fx.env.TOOLPORT_REGISTRY, JSON.stringify(registry));
      return fx;
    } catch (error) {
      await fx.close();
      throw error;
    }
  }
  session(mode) {
    const session = new Session(
      this.binary,
      {
        ...this.env,
        TOOLPORT_CLIENT_ID: mode === "full" ? "client:bench-perf" : "client:bench-search",
      },
      this.home,
    );
    this.sessions.push(session);
    return session;
  }
  async daemonPids() {
    const pids = [];
    for (const file of await readdir(this.data)) {
      if (!/^daemon-.*\.json$/.test(file)) continue;
      const { pid } = JSON.parse(await readFile(path.join(this.data, file), "utf8"));
      if (Number.isInteger(pid) && pid > 0 && (await owns(pid, this.home)))
        pids.push(pid);
    }
    return [...new Set(pids)];
  }
  close() {
    this.closing ??= this.cleanup();
    return this.closing;
  }
  async cleanup() {
    await Promise.all(this.sessions.map((session) => session.close()));
    for (const pid of await ownedPids(this.home))
      if (await owns(pid, this.home)) signal(pid, "SIGTERM");
    await sleep(500);
    for (const pid of await ownedPids(this.home))
      if (await owns(pid, this.home)) signal(pid, "SIGKILL");
    // Allow killed children to exit before deleting their fixture files.
    for (let i = 0; i < 20 && (await ownedPids(this.home)).length; i++) await sleep(50);
    assert.equal(
      (await ownedPids(this.home)).length,
      0,
      "fixture processes survived cleanup",
    );
    await rm(this.home, { recursive: true, force: true });
    active.delete(this);
  }
}

async function fullCatalog(fx, count) {
  const session = fx.session("full");
  await session.initialize();
  const deadline = performance.now() + 60_000;
  while (performance.now() < deadline) {
    const { result } = await session.request("tools/list");
    const downstream = result.tools.filter((tool) => !tool.name.startsWith("toolport_"));
    if (downstream.length === count)
      return { session, coldMs: performance.now() - session.started };
    await sleep(20);
  }
  throw new Error("full catalog incomplete");
}

async function rss(pid, field = "VmHWM") {
  const status = await readFile(`/proc/${pid}/status`, "utf8");
  const match = new RegExp(`^${field}:\\s+(\\d+) kB$`, "m").exec(status);
  assert(match, `Linux ${field} unavailable`);
  return Number(match[1]);
}

async function warm(fx, catalog, queries, calls) {
  const { session: full } = await fullCatalog(fx, catalog.length);
  const raw = {};
  async function measure(key, session, method, params) {
    const samples = [];
    let result;
    for (let i = 0; i < calls; i++) {
      const response = await session.request(
        method,
        typeof params === "function" ? params(i) : params,
      );
      result = response.result;
      samples.push(response.ms);
      if (method === "tools/call" && key !== "search_ms") {
        const reply = JSON.parse(result.content[0].text);
        assert.equal(reply.ok, true);
        assert.equal(reply.name, call.name);
      }
    }
    raw[key] = samples;
    return result;
  }
  await measure("full_list_ms", full, "tools/list");
  await measure("tools_call_ms", full, "tools/call", call);
  await full.close();
  const lazy = fx.session("lazy");
  await lazy.initialize();
  const listed = await measure("lazy_list_ms", lazy, "tools/list");
  assert(listed.tools.some((tool) => tool.name === "toolport_call_tool"));
  assert(
    listed.tools.every((tool) => tool.name.startsWith("toolport_")),
    "lazy mode leaked downstream tools",
  );
  const search = (i) => ({
    name: "toolport_search_tools",
    arguments: { query: queries[i % queries.length] },
  });
  // One unmeasured first search builds the index. Timed searches stay in this session.
  const first = await lazy.request("tools/call", search(0));
  await measure("search_ms", lazy, "tools/call", (i) => search(i + 1));
  await measure("call_tool_ms", lazy, "tools/call", {
    name: "toolport_call_tool",
    arguments: call,
  });
  const daemons = await fx.daemonPids();
  assert.equal(daemons.length, 1, "expected one fixture daemon");
  const daemonPeakKiB = await rss(daemons[0]);
  const daemonSteadyKiB = await rss(daemons[0], "VmRSS");
  const adapterPeakKiB = await rss(lazy.proc.pid);
  const downstreamCalls = (await readFile(fx.trace, "utf8"))
    .trim()
    .split("\n")
    .map(JSON.parse);
  assert.equal(
    downstreamCalls.length,
    2 * calls,
    "calls must reach the mock exactly once",
  );
  assert(
    downstreamCalls.every((entry) => `${entry.namespace}__${entry.name}` === call.name),
  );
  return {
    raw,
    searchFirstMs: first.ms,
    daemonPeakKiB,
    daemonSteadyKiB,
    adapterPeakKiB,
    lazyListBytes: bytes(listed),
    downstreamCalls: downstreamCalls.length,
  };
}

async function version(binary) {
  const home = await mkdtemp(path.join(tmpdir(), "toolport-compare-version-"));
  const cleanup = new Fixture();
  cleanup.home = home;
  cleanup.sessions = [];
  active.add(cleanup);
  try {
    const proc = spawn(binary, ["--version"], { env: environment(home), cwd: home });
    let out = "",
      err = "";
    proc.stdout.on("data", (data) => {
      out += data;
    });
    proc.stderr.on("data", (data) => {
      err += data;
    });
    const timer = setTimeout(() => proc.kill("SIGKILL"), 20_000);
    try {
      await new Promise((resolve, reject) => {
        proc.once("error", reject);
        proc.once("exit", (code) =>
          code === 0 ? resolve() : reject(new Error(`version failed: ${err}`)),
        );
      });
    } finally {
      clearTimeout(timer);
    }
    const match = /\d+\.\d+\.\d+\S*/.exec(out);
    assert(match, "gateway did not report a version");
    return match[0];
  } finally {
    await cleanup.close();
  }
}

async function main() {
  const args = process.argv.slice(2);
  if (args.includes("--help")) {
    console.log(
      "node benchmark/gateway-compare.mjs --v1 PATH --v2 PATH --out FILE [--rounds 3 --cold 5 --calls 60]",
    );
    return;
  }
  const options = { rounds: 3, cold: 5, calls: 60 };
  for (let i = 0; i < args.length; i += 2) {
    const key = args[i].replace(/^--/, "");
    assert(
      ["v1", "v2", "out", "rounds", "cold", "calls"].includes(key) && args[i + 1],
      `invalid option: ${args[i]}`,
    );
    options[key] = ["rounds", "cold", "calls"].includes(key)
      ? Number(args[i + 1])
      : args[i + 1];
  }
  assert.equal(
    process.platform,
    "linux",
    "requires Linux /proc for RSS and process cleanup",
  );
  for (const key of ["rounds", "cold", "calls"])
    assert(Number.isInteger(options[key]) && options[key] > 0, `${key} must be positive`);
  for (const key of ["v1", "v2", "out"]) assert(options[key], `--${key} is required`);
  const catalog = JSON.parse(
    await readFile(path.join(fixtureDir, "catalog.json"), "utf8"),
  );
  const queries = JSON.parse(
    await readFile(path.join(fixtureDir, "dev.json"), "utf8"),
  ).map((intent) => intent.query);
  assert.equal(catalog.length, 1707);
  assert.equal(new Set(catalog.map((tool) => tool.name.split("__")[0])).size, 18);
  const results = {};
  for (const key of ["v1", "v2"]) {
    const binary = await realpath(options[key]);
    const reported = await version(binary);
    assert(
      key === "v1" ? reported === "1.24.0" : reported.startsWith("2."),
      `unexpected ${key} version: ${reported}`,
    );
    results[key] = {
      binary,
      version: reported,
      sha256: hash(await readFile(binary)),
      cold: [],
      warm: [],
    };
  }
  for (let round = 0; round < options.rounds; round++) {
    // Alternate order between rounds to reduce load/order bias.
    for (const key of round % 2 ? ["v2", "v1"] : ["v1", "v2"]) {
      const result = results[key];
      const major = Number(result.version.split(".")[0]);
      for (let i = 0; i < options.cold; i++) {
        const fx = await Fixture.create(result.binary, major, catalog);
        try {
          result.cold.push((await fullCatalog(fx, catalog.length)).coldMs);
        } finally {
          await fx.close();
        }
      }
      const fx = await Fixture.create(result.binary, major, catalog);
      try {
        result.warm.push(await warm(fx, catalog, queries, options.calls));
      } finally {
        await fx.close();
      }
      console.error(`round ${round + 1}/${options.rounds}: ${key} complete`);
    }
  }
  for (const result of Object.values(results)) {
    result.summary = {
      cold_full_catalog_ms: summary(result.cold),
      ...Object.fromEntries(
        Object.keys(result.warm[0].raw).map((key) => [
          key,
          summary(result.warm.flatMap((round) => round.raw[key])),
        ]),
      ),
      daemon_peak_kib: summary(result.warm.map((round) => round.daemonPeakKiB)),
      daemon_steady_kib: summary(result.warm.map((round) => round.daemonSteadyKiB)),
      lazy_list_bytes: result.warm[0].lazyListBytes,
    };
    assert(
      result.warm.every(
        (round) => round.lazyListBytes === result.summary.lazy_list_bytes,
      ),
    );
  }
  const output = {
    measuredAt: new Date().toISOString(),
    node: process.version,
    machine: {
      kernel: release(),
      arch: process.arch,
      cpu: cpus()[0].model,
      logicalCpus: cpus().length,
    },
    method: {
      rounds: options.rounds,
      coldPerRound: options.cold,
      callsPerMetricPerRound: options.calls,
      catalogTools: catalog.length,
      namespaces: 18,
      catalogSha256: hash(Buffer.from(JSON.stringify(catalog))),
      queriesSha256: hash(Buffer.from(JSON.stringify(queries))),
      queries: queries.length,
      latency:
        "sequential RPC wall time including Node JSON parsing; pooled nearest-rank percentiles",
      bytes: "compact JSON result payload, excluding JSON-RPC envelope",
      rss: "daemon Linux VmHWM (peak) and VmRSS (steady) after warm metrics; excludes adapter and mocks",
    },
    results,
  };
  await writeFile(path.resolve(options.out), JSON.stringify(output, null, 2) + "\n");
  console.log(
    JSON.stringify(
      Object.fromEntries(
        Object.entries(results).map(([key, result]) => [key, result.summary]),
      ),
      null,
      2,
    ),
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  for (const name of ["SIGINT", "SIGTERM"])
    process.once(name, async () => {
      try {
        await Promise.all([...active].map((fx) => fx.close()));
      } finally {
        process.exit(name === "SIGINT" ? 130 : 143);
      }
    });
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
