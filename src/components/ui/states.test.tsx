import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EmptyState } from "./empty-state";
import { ErrorCallout } from "./status-card";
import { SkeletonRows } from "./skeleton";

vi.mock("@/lib/api", () => ({ openGatewayLog: vi.fn() }));
afterEach(() => vi.useRealTimers());

describe("list states", () => {
  it("keeps a fast load invisible and reveals three rows after 300ms", () => {
    vi.useFakeTimers();
    const { container, unmount } = render(<SkeletonRows label="Loading" />);
    const state = screen.getByRole("status");
    expect(state).toHaveClass("invisible");
    act(() => vi.advanceTimersByTime(299));
    expect(state).toHaveClass("invisible");
    act(() => vi.advanceTimersByTime(1));
    expect(state).not.toHaveClass("invisible");
    expect(container.querySelector('[data-slot="skeleton-rows"]')?.children).toHaveLength(
      3,
    );
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps retries distinct from empty content and exposes both recovery actions", () => {
    const retry = vi.fn();
    const log = vi.fn();
    const submit = vi.fn((event) => event.preventDefault());
    render(
      <form onSubmit={submit}>
        <ErrorCallout
          title="Could not load"
          cause="Offline"
          onRetry={retry}
          onViewLog={log}
        />
      </form>,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Offline");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    fireEvent.click(screen.getByRole("button", { name: "View log" }));
    expect(retry).toHaveBeenCalledOnce();
    expect(log).toHaveBeenCalledOnce();
    expect(submit).not.toHaveBeenCalled();
  });

  it("offers a working action inside an empty row group", () => {
    const add = vi.fn();
    render(
      <EmptyState
        title="No servers"
        action={<button onClick={add}>Add server</button>}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Add server" }));
    expect(add).toHaveBeenCalledOnce();
    expect(screen.getByRole("status")).toHaveTextContent("No servers");
  });
});
