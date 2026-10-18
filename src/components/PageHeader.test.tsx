import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { PageHeader, PageHeaderActions, PageHeaderActionsSlot } from "./PageHeader";

describe("PageHeader", () => {
  it("shows one title and one subtitle", () => {
    render(<PageHeader title="Skills" subtitle="Skills and rules each agent loads" />);
    expect(screen.getAllByRole("heading")).toHaveLength(1);
    expect(screen.getByRole("heading", { level: 1, name: "Skills" })).toBeInTheDocument();
    expect(screen.getByText("Skills and rules each agent loads")).toBeInTheDocument();
  });

  it("offers a back button for sub-pages", async () => {
    const onClick = vi.fn();
    render(
      <PageHeader
        title="Catalog"
        back={{ label: "Servers", ariaLabel: "Back to servers", onClick }}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Back to servers" }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("puts a view's actions in the header ahead of the shared ones", () => {
    function Page() {
      const [slot, setSlot] = useState<HTMLElement | null>(null);
      return (
        <>
          <PageHeader
            title="Activity"
            actionsSlotRef={setSlot}
            actions={<button type="button">Refresh</button>}
          />
          <PageHeaderActionsSlot value={slot}>
            <main>
              <PageHeaderActions>
                <button type="button">Export</button>
              </PageHeaderActions>
            </main>
          </PageHeaderActionsSlot>
        </>
      );
    }
    render(<Page />);
    const header = screen.getByRole("banner");
    expect(
      within(header)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Export", "Refresh"]);
    expect(within(screen.getByRole("main")).queryByRole("button")).toBeNull();
  });

  it("renders actions in place when there is no page header", () => {
    render(
      <PageHeaderActions>
        <button type="button">Export</button>
      </PageHeaderActions>,
    );
    expect(screen.getByRole("button", { name: "Export" })).toBeInTheDocument();
  });
});
