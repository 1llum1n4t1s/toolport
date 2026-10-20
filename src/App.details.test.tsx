import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import type { DetectedClient, Registry } from "@/lib/types";

const getRegistry = vi.fn();
const detectClients = vi.fn();
const takeRegistryRecoveryNotice = vi.fn();
const uninstallGateway = vi.fn();

vi.mock("@/lib/api", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api")>()),
  teamPairState: vi.fn(() => Promise.resolve(null)),
  teamPairCancel: vi.fn(),
  addServer: vi.fn(),
  detectClients: (...a: unknown[]) => detectClients(...a),
  getRegistry: (...a: unknown[]) => getRegistry(...a),
  importServers: vi.fn(),
  mainWindowVisible: vi.fn(() => Promise.resolve(true)),
  parseServerSnippet: vi.fn(),
  previewImportServers: vi.fn(),
  probeServers: vi.fn(() => Promise.resolve([])),
  uninstallGateway: (...a: unknown[]) => uninstallGateway(...a),
  recentClientActivity: vi.fn(() => Promise.resolve([])),
  removeServer: vi.fn(),
  setAllEnabled: vi.fn(),
  setSecret: vi.fn(),
  setServerEnabled: vi.fn(),
  takeRegistryRecoveryNotice: (...a: unknown[]) => takeRegistryRecoveryNotice(...a),
  teamSyncWait: vi.fn(),
  testServer: vi.fn(),
  updateServer: vi.fn(),
  popularCatalog: vi.fn(() => Promise.resolve([])),
  searchCatalog: vi.fn(() => Promise.resolve([])),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/lib/trayApprovals", () => ({
  subscribeToTrayApprovals: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("@/lib/theme", () => ({
  useTheme: () => ({ resolved: "light" }),
}));

vi.mock("@/components/AppSidebar", () => ({
  AppSidebar: ({
    onSelectView,
  }: {
    onSelectView: (view: "clients" | "servers") => void;
  }) => (
    <nav>
      <button onClick={() => onSelectView("clients")}>Clients</button>
      <button onClick={() => onSelectView("servers")}>Servers</button>
    </nav>
  ),
}));
vi.mock("@/components/PendingApprovals", () => ({ PendingApprovals: () => null }));
vi.mock("@/components/QuarantineAlert", () => ({ QuarantineAlert: () => null }));

/** A registry that already has a server, so the Servers view renders its header
 * actions instead of the empty state (which has its own catalog link). */
function registryWithServer(): Registry {
  return {
    version: 1,
    servers: [
      {
        id: "files",
        name: "Files",
        transport: "stdio",
        command: "npx",
        args: ["-y", "mcp-files"],
        env: [],
        url: null,
        source: null,
      },
    ],
    profiles: [{ id: "default", name: "Default", enabledServerIds: [] }],
    activeProfileId: "default",
  } as Registry;
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  getRegistry.mockResolvedValue(registryWithServer());
  detectClients.mockResolvedValue([]);
  takeRegistryRecoveryNotice.mockResolvedValue(null);
  uninstallGateway.mockResolvedValue({ restored: [], warnings: [] });
});

const profileClient = (id: string, connected = true): DetectedClient => ({
  id,
  name: "Claude Code",
  appPresent: true,
  configExists: true,
  configPath: `/fixture/${id}/config.json`,
  gatewayInstalled: connected,
  entryState: connected ? "managed" : "absent",
  error: null,
  usesConnectors: false,
  pluginServers: [],
  servers: [],
});

describe("pushed detail navigation", () => {
  it("opens a server page, keeps sheets transient, and returns keyboard focus to its row", async () => {
    const user = userEvent.setup();
    render(<App />);
    const open = await screen.findByRole("button", { name: "Show Files details" });
    const listViewport = document.querySelector<HTMLElement>(
      'main [data-slot="scroll-area-viewport"]',
    )!;
    listViewport.scrollTop = 400;
    open.focus();
    await user.keyboard("{Enter}");
    expect(
      await screen.findByRole("heading", { level: 1, name: "Files" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Browse catalog/ }),
    ).not.toBeInTheDocument();
    expect(
      document.querySelector<HTMLElement>('main [data-slot="scroll-area-viewport"]')
        ?.scrollTop,
    ).toBe(0);
    expect(screen.getByRole("tab", { name: "Overview" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "Files" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Back to servers" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Show Files details" })).toHaveFocus(),
    );
  });

  it("refreshes a selected server and gives a safe missing state after removal", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole("button", { name: "Show Files details" }));
    const next = registryWithServer();
    next.servers[0].args = ["new-argument"];
    getRegistry.mockResolvedValue(next);
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("npx new-argument")).toBeInTheDocument();
    getRegistry.mockResolvedValue({ ...next, servers: [] });
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("Server is no longer available")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("targets the selected client instance and refreshes detail after disconnect", async () => {
    const user = userEvent.setup();
    const first = profileClient("claude-code@personal");
    const second = profileClient("claude-code@work");
    detectClients.mockResolvedValue([first, second]);
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Clients" }));
    await waitFor(() =>
      expect(
        document.querySelector(`[data-client-id="${second.id}"]`),
      ).toBeInTheDocument(),
    );
    await user.click(
      document.querySelector<HTMLButtonElement>(`[data-client-id="${second.id}"]`)!,
    );
    expect(await screen.findByText(second.configPath)).toBeInTheDocument();
    detectClients.mockResolvedValue([first, profileClient(second.id, false)]);
    await user.click(screen.getByRole("button", { name: "Disconnect" }));
    await user.click(screen.getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(uninstallGateway).toHaveBeenCalledWith(second.id));
    expect(
      await screen.findByRole("button", { name: "Connect to Toolport" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Still connected until restart")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Back to clients" }));
    await waitFor(() =>
      expect(document.querySelector(`[data-client-id="${second.id}"]`)).toHaveFocus(),
    );
  });

  it("does not leave stale client actions after a missing instance or scan error", async () => {
    const user = userEvent.setup();
    const selected = profileClient("codex@work");
    detectClients.mockResolvedValue([selected]);
    render(<App />);
    await user.click(screen.getByRole("button", { name: "Clients" }));
    await waitFor(() =>
      expect(
        document.querySelector(`[data-client-id="${selected.id}"]`),
      ).toBeInTheDocument(),
    );
    await user.click(
      document.querySelector<HTMLButtonElement>(`[data-client-id="${selected.id}"]`)!,
    );
    detectClients.mockRejectedValue(new Error("scan failed"));
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(
      await screen.findByText("Error: scan failed", { exact: true }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Disconnect" })).not.toBeInTheDocument();
    detectClients.mockResolvedValue([]);
    await act(async () => {
      await user.click(screen.getByRole("button", { name: "Refresh" }));
    });
    expect(await screen.findByText("Client is no longer available")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Servers" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Servers" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Back to clients" }),
    ).not.toBeInTheDocument();
  });
});
