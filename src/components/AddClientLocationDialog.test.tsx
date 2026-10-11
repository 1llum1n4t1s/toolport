import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { addClientLocation, getClientLocationTypes } from "@/lib/api";
import { AddClientLocationDialog } from "./AddClientLocationDialog";
vi.mock("@/lib/api", () => ({ addClientLocation: vi.fn(), getClientLocationTypes: vi.fn() }));
beforeEach(() => {
  vi.mocked(getClientLocationTypes).mockResolvedValue([
    { id: "claude-code", name: "Claude Code", configFile: ".claude.json" },
    { id: "codex", name: "Codex", configFile: "config.toml" },
  ]);
});
it("adds a movable Codex home and selects its distinct identity", async () => {
  const onAdded = vi.fn().mockResolvedValue(undefined);
  const onOpenChange = vi.fn();
  vi.mocked(addClientLocation).mockResolvedValue("codex@work");
  render(<AddClientLocationDialog open onOpenChange={onOpenChange} onAdded={onAdded} />);
  await screen.findByRole("option", { name: "Codex" });
  await userEvent.selectOptions(screen.getByLabelText("Client type"), "codex");
  await userEvent.type(screen.getByLabelText("Config folder"), "/tmp/work-codex");
  await userEvent.click(screen.getByRole("button", { name: "Add location" }));
  expect(addClientLocation).toHaveBeenCalledWith("codex", "/tmp/work-codex");
  expect(onAdded).toHaveBeenCalledWith("codex@work");
  expect(onOpenChange).toHaveBeenCalledWith(false);
});
it("keeps the folder for correction after a validation failure", async () => {
  vi.mocked(addClientLocation).mockRejectedValue(new Error("Choose an existing client config folder"));
  render(<AddClientLocationDialog open onOpenChange={vi.fn()} onAdded={vi.fn()} />);
  await screen.findByRole("option", { name: "Claude Code" });
  await userEvent.type(screen.getByLabelText("Config folder"), "/missing");
  await userEvent.click(screen.getByRole("button", { name: "Add location" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Choose an existing client config folder");
  expect(screen.getByLabelText("Config folder")).toHaveValue("/missing");
});
