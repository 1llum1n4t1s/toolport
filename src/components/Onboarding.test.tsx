import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { Onboarding } from "./Onboarding";
import type { Registry } from "@/lib/types";

it("skips adding servers directly to the setup review", () => {
  render(
    <Onboarding
      initialStep={1}
      clients={[]}
      registry={{ servers: [], profiles: [] } as unknown as Registry}
      onRegistryChange={vi.fn()}
      onClientsRefresh={vi.fn()}
      onBrowseCatalog={vi.fn()}
      onProbe={vi.fn().mockResolvedValue([])}
      onOpenTools={vi.fn()}
      onFinish={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: /I'll add servers later/ }));
  expect(screen.getByRole("dialog")).toHaveAccessibleName("Setup started");
  expect(screen.queryByText("Connect a client")).not.toBeInTheDocument();
});
