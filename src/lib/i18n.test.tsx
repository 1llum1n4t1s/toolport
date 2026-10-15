import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

const backend = vi.hoisted(() => ({
  getUiLanguage: vi.fn<() => Promise<string>>(),
  setUiLanguage: vi.fn<(language: string) => Promise<void>>(),
}));
vi.mock("./api", () => backend);

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  vi.restoreAllMocks();
  localStorage.clear();
  backend.getUiLanguage.mockResolvedValue("system");
  backend.setUiLanguage.mockResolvedValue();
  vi.doUnmock("../locales/ja");
  vi.doUnmock("../locales/de");
});

describe("machine language bootstrap", () => {
  it("loads the authoritative saved dictionary before bootstrap resolves", async () => {
    localStorage.setItem("toolport-language", "en");
    backend.getUiLanguage.mockResolvedValue("ja");
    vi.doMock("../locales/ja", () => ({ default: { Language: "日本語" } }));
    const i18n = await import("./i18n");
    await i18n.initializeLanguage();
    expect(i18n.translate("Language")).toBe("日本語");
    expect(i18n.getLocale()).toBe("ja");
    expect(document.documentElement.lang).toBe("ja");
    expect(localStorage.getItem("toolport-language")).toBe("ja");
  });

  it("loads the system dictionary before bootstrap resolves", async () => {
    vi.spyOn(navigator, "languages", "get").mockReturnValue(["de-DE"]);
    vi.doMock("../locales/de", () => ({ default: { Language: "Sprache" } }));
    const i18n = await import("./i18n");
    await i18n.initializeLanguage();
    expect(i18n.translate("Language")).toBe("Sprache");
    expect(i18n.getLocale()).toBe("de");
  });

  it("uses English formatting and text when a dictionary fails", async () => {
    backend.getUiLanguage.mockResolvedValue("ja");
    vi.doMock("../locales/ja", () => {
      throw new Error("dictionary unavailable");
    });
    const i18n = await import("./i18n");
    await i18n.initializeLanguage();
    expect(i18n.getLocale()).toBe("en");
    expect(i18n.translate("Language")).toBe("Language");
    expect(document.documentElement.lang).toBe("en");
  });

  it("keeps the latest selection when earlier loads or writes finish late", async () => {
    let finishSave!: () => void;
    backend.setUiLanguage.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishSave = resolve;
        }),
    );
    let finishDictionary!: (value: { default: Record<string, string> }) => void;
    vi.doMock(
      "../locales/ja",
      () =>
        new Promise((resolve) => {
          finishDictionary = resolve;
        }),
    );
    vi.doMock("../locales/de", () => ({ default: { Language: "Sprache" } }));
    const i18n = await import("./i18n");
    const first = i18n.setLanguage("ja");
    const second = i18n.setLanguage("de");
    await vi.waitFor(() => expect(finishSave).toBeTypeOf("function"));
    expect(backend.setUiLanguage.mock.calls).toEqual([["ja"]]);
    finishSave();
    finishDictionary({ default: { Language: "日本語" } });
    await Promise.all([first, second]);
    expect(backend.setUiLanguage.mock.calls).toEqual([["ja"], ["de"]]);
    expect(i18n.getLocale()).toBe("de");
    expect(i18n.translate("Language")).toBe("Sprache");
    expect(localStorage.getItem("toolport-language")).toBe("de");
  });
  it("surfaces persistence failures without caching an unsaved choice", async () => {
    backend.getUiLanguage.mockResolvedValue("en");
    backend.setUiLanguage.mockRejectedValue(new Error("disk full"));
    const i18n = await import("./i18n");
    await i18n.initializeLanguage();
    const { result } = renderHook(() => i18n.useI18n());
    await act(async () => {
      await i18n.setLanguage("en");
    });
    expect(result.current.saveError).toBe(true);
    expect(result.current.error).toBe(false);
    expect(localStorage.getItem("toolport-language")).toBe("en");
  });
});
