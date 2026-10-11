import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { QuarantineDefinition } from "./QuarantineDefinition";
import type { QuarantinedTool } from "@/lib/api";

const getQuarantineDefinition = vi.fn();
vi.mock("@/lib/api", () => ({
  getQuarantineDefinition: (...args: unknown[]) => getQuarantineDefinition(...args),
}));
const tool: QuarantinedTool = {
  record_id: "reviewed-record",
  server: "srv",
  tool: "srv__wipe",
  reason: "poisoned definition",
  profile: "work",
  ts: 1,
  definition: { name: "srv__wipe", description: "Captured text" },
};

describe("QuarantineDefinition", () => {
  it("formats no closed row definitions, even for thousands of records", () => {
    const stringify = vi.spyOn(JSON, "stringify");
    render(
      <>
        {Array.from({ length: 3000 }, (_, i) => (
          <QuarantineDefinition key={i} tool={tool} />
        ))}
      </>,
    );
    expect(
      stringify.mock.calls.filter(([value]) => value === tool.definition),
    ).toHaveLength(0);
    stringify.mockRestore();
    expect(screen.queryByText(/Captured text/)).not.toBeInTheDocument();
  });

  it("fetches only an expanded definition and binds the fetch to the record", async () => {
    getQuarantineDefinition.mockResolvedValue(tool.definition);
    const summary = { ...tool, definition: undefined, has_definition: true };
    render(<QuarantineDefinition tool={summary} />);
    expect(getQuarantineDefinition).not.toHaveBeenCalled();
    const details = screen.getByText("Captured definition").closest("details")!;
    details.open = true;
    fireEvent(details, new Event("toggle"));
    await waitFor(() =>
      expect(getQuarantineDefinition).toHaveBeenCalledWith(
        "work",
        "srv__wipe",
        "reviewed-record",
      ),
    );
    expect(await screen.findByText(/Captured text/)).toBeInTheDocument();
  });
});
