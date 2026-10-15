#!/usr/bin/env node
/* global window, document, MutationObserver */
import { chromium, expect } from "@playwright/test";
import { createServer } from "vite";
import { existsSync, realpathSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = path.join(
  process.env.TOOLPORT_VERIFY_DIR || path.join(root, ".verify"),
  `localization-${Date.now()}-${process.pid}`,
);
await mkdir(output, { recursive: true });
const languages = ["en", "ja", "zh-CN", "zh-TW", "ko", "es", "fr", "de", "pt-BR"];
const catalogs = {};
for (const language of languages) {
  const groups = await Promise.all(
    ["shell", "connections", "workflows", "common"].map(async (group) =>
      JSON.parse(
        await readFile(path.join(root, "src/locales", group, `${language}.json`), "utf8"),
      ),
    ),
  );
  catalogs[language] = Object.assign({}, ...groups);
}
const text = (language, key) => catalogs[language][key] ?? key;
const errors = [];
const server = await createServer({
  root,
  server: {
    host: "127.0.0.1",
    port: 0,
    open: false,
    hmr: false,
    fs: { allow: [root, realpathSync(path.join(root, "node_modules"))] },
  },
  logLevel: "error",
});
let browser;
let context;
let page;
try {
  await server.listen();
  const baseURL = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({
    executablePath:
      process.env.TOOLPORT_BROWSER_BIN ||
      (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined),
    headless: true,
  });
  async function open({
    locale = "en-US",
    saved = "system",
    blocked = false,
    query = "localization-controls",
    delayLanguage,
    failLanguage,
    watchKey = "Language",
  } = {}) {
    if (context) await context.close();
    context = await browser.newContext({
      locale,
      viewport: { width: 1440, height: 1000 },
    });
    await context.addInitScript(
      ({ saved, blocked, watchKey }) => {
        // Session storage emulates the backend; localStorage is only React's cache.
        if (sessionStorage.getItem("toolport-fixture-language") === null)
          sessionStorage.setItem("toolport-fixture-language", saved);
        localStorage.setItem("toolport.onboarded", "1");
        if (blocked) {
          const get = Storage.prototype.getItem;
          const set = Storage.prototype.setItem;
          Storage.prototype.getItem = function (key) {
            if (this === localStorage && key === "toolport-language")
              throw new DOMException("Storage blocked", "SecurityError");
            return get.call(this, key);
          };
          Storage.prototype.setItem = function (key, value) {
            if (this === localStorage && key === "toolport-language")
              throw new DOMException("Storage blocked", "SecurityError");
            return set.call(this, key, value);
          };
        }
        window.localizationPaints = [];
        const observer = new MutationObserver(() => {
          const root = document.getElementById("root");
          if (!root?.textContent?.trim()) return;
          const heading = root.querySelector('[data-testid="language-heading"]');
          const content = heading?.textContent || root.textContent;
          window.localizationPaints.push({
            text: content,
            lang: document.documentElement.lang,
            english: heading
              ? content === watchKey
              : new RegExp(`\\b${watchKey}\\b`).test(content),
          });
        });
        observer.observe(document, {
          childList: true,
          subtree: true,
          characterData: true,
        });
      },
      { saved, blocked, watchKey },
    );
    page = await context.newPage();
    page.on("pageerror", (error) => errors.push(error.message));
    await page.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== baseURL) return route.abort();
      if (failLanguage && url.pathname === `/src/locales/${failLanguage}.ts`)
        return route.abort();
      if (delayLanguage && url.pathname === `/src/locales/${delayLanguage}.ts`)
        await new Promise((resolve) => setTimeout(resolve, 350));
      return route.continue();
    });
    await page.goto(`${baseURL}/fixtures/?${query}`);
    return page;
  }
  async function selected(language) {
    await expect(page.locator("html")).toHaveAttribute("lang", language);
    await expect(page.locator("#fixture-language")).toHaveAttribute("aria-busy", "false");
    await expect(page.getByTestId("language-heading")).toHaveText(
      text(language, "Language"),
    );
  }
  async function noEnglishPaint(language) {
    await expect
      .poll(() => page.evaluate(() => window.localizationPaints.length))
      .toBeGreaterThan(0);
    const paints = await page.evaluate(() => window.localizationPaints);
    expect(
      paints.every((paint) => paint.lang === language && !paint.english),
      JSON.stringify(paints),
    ).toBe(true);
  }
  // Every supported choice loads, preserves unsaved state, persists and reloads.
  await open();
  await selected("en");
  await page.getByTestId("draft").fill("Draft <keep> & client");
  for (const language of languages) {
    await page.locator("#fixture-language").selectOption(language);
    await selected(language);
    await expect(page.getByTestId("draft")).toHaveValue("Draft <keep> & client");
    await expect
      .poll(() =>
        page.evaluate(() => sessionStorage.getItem("toolport-fixture-language")),
      )
      .toBe(language);
    await expect
      .poll(() => page.evaluate(() => localStorage.getItem("toolport-language")))
      .toBe(language);
    await expect(page.getByTestId("execution-review")).toContainText(
      text(language, "New server"),
    );
  }
  await page.reload();
  await selected("pt-BR");
  expect(await page.evaluate(() => window.toolportFixture.missing)).toEqual([]);

  // Assert the first actual UI mutation, after a delayed dictionary load. Both
  // machine-saved and system-derived languages must be ready before mounting.
  for (const scenario of [
    { locale: "en-US", saved: "ja", language: "ja" },
    { locale: "de-DE", saved: "system", language: "de" },
  ]) {
    await open({ ...scenario, delayLanguage: scenario.language });
    await selected(scenario.language);
    await noEnglishPaint(scenario.language);
  }
  for (const [locale, language] of [
    ["zh-HK", "zh-TW"],
    ["it-IT", "en"],
  ]) {
    await open({ locale, saved: "invalid-choice" });
    await selected(language);
    await expect(page.locator("#fixture-language")).toHaveValue("system");
  }
  await open({ saved: "ja", blocked: true });
  await selected("ja");
  await page.locator("#fixture-language").selectOption("fr");
  await selected("fr");
  await page.reload();
  await selected("fr");

  // A failed dictionary changes formatting as well as text to English. Reload
  // retries the saved machine language once the network failure is removed.
  await open({ saved: "de", failLanguage: "de" });
  await expect(page.getByTestId("dictionary-error")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.getByTestId("language-heading")).toHaveText("Language");
  await expect(page.getByTestId("number")).toHaveText(
    new Intl.NumberFormat("en").format(1234.56),
  );
  await expect(page.getByTestId("date")).toHaveText(
    new Intl.DateTimeFormat("en", { timeZone: "UTC", dateStyle: "long" }).format(
      new Date("2026-10-14T12:00:00Z"),
    ),
  );
  await page.unroute("**/*");
  await page.route("**/*", (route) =>
    new URL(route.request().url()).origin === baseURL ? route.continue() : route.abort(),
  );
  await page.reload();
  await selected("de");

  await open({ query: "localization-controls&language-save-error", saved: "en" });
  await page.locator("#fixture-language").selectOption("ja");
  await selected("ja");
  await expect(page.getByTestId("save-error")).toBeVisible();
  expect(
    await page.evaluate(() => sessionStorage.getItem("toolport-fixture-language")),
  ).toBe("en");

  // Capture real 2.0 screens in the requested languages, and exercise the member
  // review and personal sync fixtures without touching native state.
  for (const language of ["ja", "de", "zh-TW"]) {
    await open({
      saved: language,
      query: "",
      delayLanguage: language,
      watchKey: "Servers",
    });
    await expect(page.locator("html")).toHaveAttribute("lang", language);
    await expect(page.getByText("GitHub", { exact: true }).first()).toBeVisible();
    await noEnglishPaint(language);
    await page.screenshot({ path: path.join(output, `${language}-main.png`) });
    await page
      .getByRole("button", { name: text(language, "Settings"), exact: true })
      .click();
    await expect(page.locator("#interface-language")).toBeVisible();
    await page.screenshot({ path: path.join(output, `${language}-settings.png`) });
    await page
      .getByRole("button", { name: text(language, "Activity"), exact: true })
      .click();
    await page.screenshot({ path: path.join(output, `${language}-activity.png`) });
    expect(await page.evaluate(() => window.toolportFixture.missing)).toEqual([]);
    for (const query of ["teams-review", "personal-sync=trial"]) {
      await page.goto(`${baseURL}/fixtures/?${query}`);
      await expect(page.locator("html")).toHaveAttribute("lang", language);
      await expect(page.locator("#root main")).toBeVisible();
      await page.screenshot({
        path: path.join(output, `${language}-${query.split("=")[0]}.png`),
      });
      expect(await page.evaluate(() => window.toolportFixture.missing)).toEqual([]);
    }
  }
  expect(errors).toEqual([]);
  await writeFile(
    path.join(output, "summary.json"),
    JSON.stringify(
      {
        languages,
        screenshots: ["ja", "de", "zh-TW"],
        checks: [
          "cold saved and system first paint",
          "persistence",
          "blocked cache",
          "dictionary recovery",
          "English formatting fallback",
          "unsaved input",
          "save error",
          "2.0 review and sync",
        ],
      },
      null,
      2,
    ),
  );
  console.log(
    `Localization smoke passed: 9 languages, first paint, persistence, fallback and 2.0 fixtures. Screenshots: ${output}`,
  );
} catch (error) {
  if (page) {
    await page.screenshot({ path: path.join(output, "failure.png") }).catch(() => {});
    await writeFile(path.join(output, "failure.html"), await page.content()).catch(
      () => {},
    );
  }
  throw error;
} finally {
  if (context) await context.close();
  if (browser) await browser.close();
  await server.close();
}
