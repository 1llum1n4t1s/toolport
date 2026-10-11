// Offline namespace server for gateway-compare.mjs. No provider calls.
import { appendFileSync, readFileSync } from "node:fs";
import { createInterface } from "node:readline";

const [catalogPath, namespace, trace] = process.argv.slice(2);
const tools = JSON.parse(readFileSync(catalogPath, "utf8"))
  .filter((tool) => tool.name.startsWith(`${namespace}__`))
  .map((tool) => ({ ...tool, name: tool.name.slice(namespace.length + 2) }));
const names = new Set(tools.map((tool) => tool.name));
for await (const line of createInterface({ input: process.stdin })) {
  const request = JSON.parse(line);
  if (request.id === undefined) continue;
  let result;
  switch (request.method) {
    case "initialize":
      result = {
        protocolVersion: request.params.protocolVersion,
        capabilities: { tools: {} },
        serverInfo: { name: "public-catalog-fixture", version: "1" },
      };
      break;
    case "tools/list":
      result = { tools };
      break;
    case "tools/call": {
      const known = names.has(request.params.name);
      appendFileSync(trace, JSON.stringify({ namespace, ...request.params }) + "\n");
      result = {
        ...(known ? {} : { isError: true }),
        content: [
          {
            type: "text",
            text: JSON.stringify({
              ok: known,
              name: `${namespace}__${request.params.name}`,
              value: 1,
              url: "https://example.test/issues/42",
              rows: [],
              marker: "fixture-complete",
            }),
          },
        ],
      };
      break;
    }
    case "resources/list":
      result = { resources: [] };
      break;
    case "resources/templates/list":
      result = { resourceTemplates: [] };
      break;
    case "prompts/list":
      result = { prompts: [] };
      break;
    case "ping":
      result = {};
      break;
    default:
      process.stdout.write(
        JSON.stringify({
          jsonrpc: "2.0",
          id: request.id,
          error: { code: -32601, message: "Unknown fixture method" },
        }) + "\n",
      );
      continue;
  }
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }) + "\n");
}
