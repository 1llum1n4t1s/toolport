import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import {
  dismissSkillIssue,
  fixSkillIssue,
  openSkillPath,
  openSkillPathWith,
  removeSkill,
  setSkillState,
  skillOpenApps,
  skillsInventory,
} from "@/lib/api";
import { applySkillState, skillsFixture } from "@/test/skills-fixture";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";
import { toastError } from "@/lib/toast";
import { SKILLS_REFRESH_EVENT } from "@/lib/shortcuts";
import { SkillsView } from "./SkillsView";

vi.mock("@/lib/api", () => ({
  skillsInventory: vi.fn(),
  setSkillState: vi.fn(),
  fixSkillIssue: vi.fn(),
  openSkillPath: vi.fn(),
  openSkillPathWith: vi.fn(),
  skillOpenApps: vi.fn(),
  removeSkill: vi.fn(),
  dismissSkillIssue: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("@/lib/toast", () => ({ toastError: vi.fn() }));

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
  vi.mocked(openSkillPathWith).mockResolvedValue(undefined);
  vi.mocked(skillOpenApps).mockResolvedValue([
    { id: "obsidian.desktop", name: "Obsidian", default: true, chosen: false },
    { id: "dev.zed.Zed.desktop", name: "Zed", default: false, chosen: false },
  ]);
  vi.mocked(removeSkill).mockImplementation(async (key) => {
    const inv = skillsFixture();
    return {
      trashed: "~/.agents/skills/triage",
      links: 1,
      inventory: { ...inv, skills: inv.skills.filter((s) => s.key !== key) },
    };
  });
  vi.mocked(dismissSkillIssue).mockImplementation(async (fingerprint, dismissed) => {
    const inv = skillsFixture();
    return {
      ...inv,
      issues: inv.issues.map((i) =>
        i.fingerprint === fingerprint ? { ...i, dismissed } : i,
      ),
    };
  });
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
    // Only your own skills start open; other groups are one click away.
    expect(within(list).queryByRole("button", { name: /pdf/ })).not.toBeInTheDocument();
    await userEvent.click(
      within(list).getByRole("button", { name: /claude\.ai account/ }),
    );
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

  it("keeps the switch and rescans when a client refuses the write", async () => {
    vi.mocked(setSkillState).mockRejectedValueOnce(
      new Error("permissions isn't in a shape"),
    );
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(within(list).getByRole("button", { name: /screenshot/ }));
    const details = screen.getByRole("complementary", { name: "Skill details" });
    const loadsBefore = vi.mocked(skillsInventory).mock.calls.length;
    await userEvent.click(
      within(details).getByRole("switch", { name: "Load screenshot in OpenCode" }),
    );
    await waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(
      within(details).getByRole("switch", { name: "Load screenshot in OpenCode" }),
    ).toBeChecked();
    await waitFor(() =>
      expect(vi.mocked(skillsInventory).mock.calls.length).toBeGreaterThan(loadsBefore),
    );
  });

  it("keeps the confirm open when a fix fails", async () => {
    vi.mocked(fixSkillIssue).mockRejectedValueOnce(new Error("disk full"));
    renderView();
    await userEvent.click(await screen.findByRole("tab", { name: /Needs a look/ }));
    await userEvent.click(screen.getByRole("button", { name: "Remove link" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove link" }));
    await waitFor(() => expect(toastError).toHaveBeenCalled());
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("moves between tabs with the arrow keys and clears an agent filter", async () => {
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(
      within(list).getByRole("button", { name: "Show only what Codex loads" }),
    );
    const skillsTab = screen.getByRole("tab", { name: /Skills/ });
    skillsTab.focus();
    await userEvent.keyboard("{ArrowRight}");
    const rulesTab = screen.getByRole("tab", { name: /Rules/ });
    expect(rulesTab).toHaveAttribute("aria-selected", "true");
    expect(rulesTab).toHaveFocus();
    expect(screen.getByRole("tabpanel")).toHaveAttribute("aria-labelledby", rulesTab.id);
    await userEvent.keyboard("{ArrowLeft}");
    const again = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(
      within(again).getByRole("button", { name: /claude\.ai account/ }),
    );
    // The Codex filter from before is gone, so skills Codex doesn't load are back.
    expect(within(again).getByRole("button", { name: /pdf/ })).toBeInTheDocument();
  });

  it("drops a write result that returns after the user picked another project", async () => {
    let resolveWrite: (value: ReturnType<typeof skillsFixture>) => void = () => {};
    vi.mocked(setSkillState).mockImplementationOnce(
      () => new Promise((resolve) => (resolveWrite = resolve)),
    );
    vi.mocked(openFolderDialog).mockResolvedValueOnce("/work/repo");
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(within(list).getByRole("button", { name: /screenshot/ }));
    const details = screen.getByRole("complementary", { name: "Skill details" });
    await userEvent.click(
      within(details).getByRole("switch", { name: "Load screenshot in OpenCode" }),
    );
    vi.mocked(skillsInventory).mockResolvedValue({
      ...skillsFixture(),
      project: "/work/repo",
    });
    await userEvent.click(screen.getByRole("button", { name: /Global only/ }));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: /Add a project folder/ }),
    );
    await screen.findByRole("button", { name: /Global \+ repo/ });
    expect(localStorage.getItem("toolport-skills-project")).toBe("/work/repo");
    // The old global write lands late; the project view must not be replaced by it.
    resolveWrite(applySkillState(skillsFixture(), "opencode", "x", "off"));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Global \+ repo/ })).toBeInTheDocument(),
    );
    expect(
      within(screen.getByRole("complementary", { name: "Skill details" })).getByRole(
        "switch",
        {
          name: "Load screenshot in OpenCode",
        },
      ),
    ).toBeChecked();
  });

  it("opens and reveals files by their full path, not the ~ form it shows", async () => {
    renderView();
    const details = await screen.findByRole("complementary", { name: "Skill details" });
    await userEvent.click(
      within(details).getByRole("button", { name: /Open SKILL\.md/ }),
    );
    expect(openSkillPath).toHaveBeenCalledWith(
      "/home/you/.agents/skills/release-notes/SKILL.md",
      false,
      null,
    );
    await userEvent.click(screen.getByRole("tab", { name: /Rules/ }));
    await userEvent.click(screen.getByRole("button", { name: /CLAUDE\.md/ }));
    const rule = screen.getByRole("complementary", { name: "Rules file details" });
    await userEvent.click(within(rule).getByRole("button", { name: /Reveal/ }));
    expect(openSkillPath).toHaveBeenLastCalledWith(
      "/home/you/.claude/CLAUDE.md",
      true,
      null,
    );
  });

  it("opens a file with the app picked from the Open menu", async () => {
    renderView();
    const details = await screen.findByRole("complementary", { name: "Skill details" });
    await userEvent.click(within(details).getByRole("button", { name: "Open with" }));
    expect(
      await screen.findByRole("menuitemradio", { name: "Obsidian (default)" }),
    ).toBeChecked();
    await userEvent.click(screen.getByRole("menuitemradio", { name: "Zed" }));
    expect(openSkillPathWith).toHaveBeenCalledWith(
      "/home/you/.agents/skills/release-notes/SKILL.md",
      "dev.zed.Zed.desktop",
      null,
    );
  });

  it("removes one of your skills after a confirm, and offers nothing for others", async () => {
    renderView();
    const list = await screen.findByRole("region", { name: "Skills" });
    await userEvent.click(within(list).getByRole("button", { name: /triage/ }));
    const details = screen.getByRole("complementary", { name: "Skill details" });
    await userEvent.click(within(details).getByRole("button", { name: "Remove triage" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Remove" }));
    expect(removeSkill).toHaveBeenCalledWith(
      "/home/you/.agents/skills/triage/SKILL.md",
      null,
    );
    await waitFor(() =>
      expect(
        within(list).queryByRole("button", { name: /triage/ }),
      ).not.toBeInTheDocument(),
    );
    await userEvent.click(
      within(list).getByRole("button", { name: /claude\.ai account/ }),
    );
    await userEvent.click(within(list).getByRole("button", { name: /pdf/ }));
    expect(
      within(screen.getByRole("complementary", { name: "Skill details" })).queryByRole(
        "button",
        { name: "Remove pdf" },
      ),
    ).not.toBeInTheDocument();
  });

  it("dismisses an item until it changes and can bring it back", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("tab", { name: /Needs a look/ }));
    const card = screen
      .getByText("Grok reads your rules file but not what it imports")
      .closest("article") as HTMLElement;
    await userEvent.click(within(card).getByRole("button", { name: "Dismiss" }));
    expect(dismissSkillIssue).toHaveBeenCalledWith("rules-imports:00000003", true, null);
    const tab = screen.getByRole("tab", { name: /Needs a look/ });
    await waitFor(() => expect(tab).toHaveTextContent("2"));
    await userEvent.click(screen.getByRole("button", { name: /Dismissed/ }));
    const back = screen
      .getByText("Grok reads your rules file but not what it imports")
      .closest("article") as HTMLElement;
    await userEvent.click(within(back).getByRole("button", { name: "Show again" }));
    expect(dismissSkillIssue).toHaveBeenLastCalledWith(
      "rules-imports:00000003",
      false,
      null,
    );
  });

  it("jumps from a skill chip on Needs a look to that skill", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("tab", { name: /Needs a look/ }));
    await userEvent.click(screen.getByRole("button", { name: "screenshot" }));
    expect(screen.getByRole("tab", { name: /Skills/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(
      within(screen.getByRole("complementary", { name: "Skill details" })).getByRole(
        "heading",
        { name: "screenshot" },
      ),
    ).toBeInTheDocument();
  });

  it("rescans when the app's Refresh shortcut fires", async () => {
    renderView();
    await screen.findByRole("region", { name: "Skills" });
    const before = vi.mocked(skillsInventory).mock.calls.length;
    window.dispatchEvent(new Event(SKILLS_REFRESH_EVENT));
    await waitFor(() =>
      expect(vi.mocked(skillsInventory).mock.calls.length).toBe(before + 1),
    );
  });
});
