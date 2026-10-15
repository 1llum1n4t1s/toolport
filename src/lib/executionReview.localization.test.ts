import { describe, expect, it, vi } from "vitest";
import type { ServerEntry } from "./types";

vi.mock("./i18n", () => ({
  msg: (source: string) => source,
  translate: (source: string, values: Record<string, string | number> = {}) => {
    const german: Record<string, string> = {
      Command: "Befehl",
      Environment: "Umgebung",
      "Working folder": "Arbeitsordner",
      "Uses this machine's environment": "Umgebung dieses Rechners verwenden",
      yes: "ja",
      no: "nein",
      Removed: "Entfernt",
      "Set on this machine": "Auf diesem Rechner festlegen",
      "<masked secret>": "<verborgenes Geheimnis>",
      "Value for {flag} (argument {position})": "Wert für {flag} (Argument {position})",
      "Secret argument {position}": "Geheimes Argument {position}",
      "{provider} entry {reference} will be sent to {destination} ({field})":
        "{provider}-Eintrag {reference} wird an {destination} gesendet ({field})",
    };
    return (german[source] ?? source).replace(/\{(\w+)\}/g, (token, key: string) =>
      String(values[key] ?? token),
    );
  },
}));
import {
  executionReviewFields,
  executionReviewLines,
  inputDisplayName,
} from "./executionReview";
import { secretReferenceReview } from "./secretRefs";

const base: ServerEntry = {
  id: "review",
  name: "Review",
  transport: "stdio",
  source: "manual",
  command: "yes",
  args: [],
  cwd: "Removed",
  inheritEnv: true,
  url: null,
  env: [{ key: "STATUS", secret: false, value: "Removed" }],
};

describe("localized consent display preserves executable values", () => {
  it("translates semantic labels without changing command, path or environment values", () => {
    for (const raw of [
      "yes",
      "no",
      "Removed",
      "Set on this machine",
      "<masked secret>",
      "Uses environment: PATH",
      "Password manager: not-a-reference",
      "Argument 1 = no",
    ]) {
      const server = {
        ...base,
        command: raw,
        env: [{ key: "STATUS", secret: false, value: raw }],
      };
      const lines = executionReviewLines(server);
      expect(lines).toContain(`Befehl: ${raw}`);
      expect(lines).toContain(`Umgebung: STATUS = ${raw}`);
      expect(lines).toContain("Arbeitsordner: Removed");
      expect(lines).toContain("Umgebung dieses Rechners verwenden: ja");
      expect(executionReviewFields(server).Command).toBe(raw);
      expect(executionReviewFields(server)["Environment: STATUS"]).toBe(raw);
    }
  });

  it("translates Removed only when the field was actually removed", () => {
    const server = { ...base, env: [], syncExecutionReview: executionReviewFields(base) };
    expect(executionReviewLines(server)).toContain("Umgebung: STATUS = Entfernt");
  });

  it("localizes reference consent wording and keeps its identifiers verbatim", () => {
    const server = {
      ...base,
      env: [
        {
          key: "TOKEN",
          secret: true,
          value: null,
          source: { ref: "op://Private/Removed/no" },
        },
      ],
    };
    expect(secretReferenceReview(server)).toEqual([
      '1Password-Eintrag "op://Private/Removed/no" wird an yes gesendet (env:TOKEN)',
    ]);
  });

  it("localizes generated input names while preserving custom saved labels", () => {
    const server = {
      ...base,
      args: ["--token", "<launch-input>"],
      launch: {
        inputs: [
          {
            key: "token",
            label: "Imported argument 2",
            secret: true,
            required: true,
            value: null,
          },
          { key: "custom", label: "Removed", secret: true, required: true, value: null },
        ],
        bindings: [{ index: 1, parts: [{ kind: "input" as const, key: "token" }] }],
      },
    };
    expect(inputDisplayName(server, "token")).toBe("Wert für --token (Argument 2)");
    expect(inputDisplayName(server, "custom")).toBe("Removed");
    expect(server.launch.inputs[0].label).toBe("Imported argument 2");
  });
});
