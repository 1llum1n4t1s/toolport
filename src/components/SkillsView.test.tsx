import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import { fixSkillIssue, openSkillPath, setSkillState, skillsInventory } from "@/lib/api";
import { applySkillState, skillsFixture } from "@/test/skills-fixture";
import { SkillsView } from "./SkillsView";

vi.mock("@/lib/api", () => ({
  skillsInventory: vi.fn(),
  setSkillState: vi.fn(),
  fixSkillIssue: vi.fn(),
  openSkillPath: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

beforeEach(() => {
  localStorage.clear();
  vi.mocked(skillsInventory).mockResolvedValue(skillsFixture());
  vi.mocked(setSkillState).mockImplementation(async (agent, key, state) =>
    applySkillState(skillsFixture(), agent, key, state),
  );
  vi.mocked(fixSkillIssue).mockImplementation(async (issue) => {
    const inv = skillsFixture();
    return { ...inv, issues: inv.issues.filter((i) => i.id !== issue) };
  });
  vi.mocked(openSkillPath).mockResolvedValue(undefined);
});

function renderView() {
  return render(
    <TooltipProvider>
      <SkillsView />
    </TooltipProvider>,
  );
}

describe("SkillsView", () => {
  it("shows each skill's state per agent and the first skill's details", async () => {
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    const row = within(list).getByRole("button", { name: /screenshot/ });
    expect(
      within(row).getByRole("img", { name: "Claude Code: Off" }),
    ).toBeInTheDocument();
    expect(within(row).getByRole("img", { name: "Cursor: On" })).toBeInTheDocument();
    const pdf = within(list).getByRole("button", { name: /pdf/ });
    expect(
      within(pdf).getByRole("img", { name: "Grok: Another copy wins" }),
    ).toBeInTheDocument();
    expect(
      within(pdf).getByRole("img", { name: "Codex: Not loaded" }),
    ).toBeInTheDocument();
    const details = screen.getByRole("complementary", { name: "Skill details" });
    expect(
      within(details).getByRole("heading", { name: "release-notes" }),
    ).toBeInTheDocument();
    expect(screen.getByText("3 things need a look")).toBeInTheDocument();
  });

  it("filters to one agent from its column", async () => {
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(
      within(list).getByRole("button", { name: "Show only what Codex loads" }),
    );
    expect(within(list).queryByRole("button", { name: /pdf/ })).not.toBeInTheDocument();
    expect(within(list).getByRole("button", { name: /triage/ })).toBeInTheDocument();
  });

  it("writes a switch through the client's own setting and shows the result", async () => {
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(within(list).getByRole("button", { name: /screenshot/ }));
    const details = screen.getByRole("complementary", { name: "Skill details" });
    await userEvent.click(
      within(details).getByRole("switch", { name: "Load screenshot in OpenCode" }),
    );
    expect(setSkillState).toHaveBeenCalledWith(
      "opencode",
      "/home/you/.agents/skills/screenshot/SKILL.md",
      "off",
      null,
    );
    expect(
      await within(details).findByRole("switch", { name: "Load screenshot in OpenCode" }),
    ).not.toBeChecked();
  });

  it("offers Claude Code's name-only state and leaves agents without a switch read-only", async () => {
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(within(list).getByRole("button", { name: /triage/ }));
    const details = screen.getByRole("complementary", { name: "Skill details" });
    const claude = within(details).getByRole("radiogroup", {
      name: "triage in Claude Code",
    });
    expect(within(claude).getByRole("radio", { name: "Name only" })).toBeChecked();
    expect(
      within(details).queryByRole("switch", { name: "Load triage in Cursor" }),
    ).not.toBeInTheDocument();
  });

  it("shows how each agent reaches a rules file, including skipped imports", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("tab", { name: /Rules/ }));
    await userEvent.click(screen.getByRole("button", { name: /CLAUDE\.md/ }));
    const details = screen.getByRole("complementary", { name: "Rules file details" });
    expect(
      within(details).getByText("skips @import ~/.agents/working-agreements.md"),
    ).toBeInTheDocument();
    expect(
      within(details).getByText(
        "Cursor keeps user rules in its app settings, not a file.",
      ),
    ).toBeInTheDocument();
  });

  it("confirms before applying a fix", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("tab", { name: /Needs a look/ }));
    await userEvent.click(screen.getByRole("button", { name: "Remove link" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove link" }));
    expect(fixSkillIssue).toHaveBeenCalledWith("broken-links", null);
    await waitFor(() =>
      expect(screen.queryByText("1 link to a missing folder")).not.toBeInTheDocument(),
    );
  });
});
