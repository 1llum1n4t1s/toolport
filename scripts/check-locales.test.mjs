import assert from "node:assert/strict";
import test from "node:test";
import ts from "typescript";
import {
  checkCatalogs,
  checkRustSource,
  checkSource,
  isMarkedType,
  parseCatalog,
} from "./check-locales.mjs";

const messages = new Set(["Hello", "Welcome {name}", "Ready"]);
const sourceErrors = (source) => checkSource(source, "fixture.tsx", messages);

test("JSON duplicate keys are rejected before JSON.parse can overwrite them", () => {
  const errors = [];
  parseCatalog('{"Hello":"Hello","Hello":"different"}', "common/en", errors);
  assert.match(errors.join("\n"), /duplicate key Hello/);
});

test("malformed catalogs and non-string values are rejected", () => {
  for (const source of ["[]", "null", "{", '{"Hello":3}', '{"Hello":""}']) {
    const errors = [];
    parseCatalog(source, "common/en", errors);
    assert.ok(errors.length, source);
  }
});

test("duplicates across groups are rejected for every locale, including unknown keys", () => {
  const catalogs = {
    common: { en: { Hello: "Hello" }, ja: { Hello: "こんにちは", extra: "余分" } },
    shell: { en: { Ready: "Ready" }, ja: { Ready: "準備完了", extra: "その他" } },
  };
  const { errors } = checkCatalogs(catalogs, ["en", "ja"], ["common", "shell"]);
  assert.ok(
    errors.some((error) => /shell\/ja: duplicate key across groups: extra/.test(error)),
  );
  catalogs.shell.en.Hello = "Hello";
  assert.match(
    checkCatalogs(catalogs, ["en", "ja"], ["common", "shell"]).errors.join("\n"),
    /shell\/en: duplicate key across groups: Hello/,
  );
});

test("catalog parity and interpolation names and occurrence counts are checked", () => {
  const catalogs = {
    common: {
      en: { "Welcome {name}": "Welcome {name}", Ready: "Ready" },
      ja: { "Welcome {name}": "ようこそ {other}", extra: "追加" },
    },
  };
  const errors = checkCatalogs(catalogs, ["en", "ja"], ["common"]).errors.join("\n");
  assert.match(errors, /invalid placeholders/);
  assert.match(errors, /missing Ready/);
  assert.match(errors, /unknown key extra/);
  catalogs.common.ja["Welcome {name}"] = "{name} {name}";
  assert.match(
    checkCatalogs(catalogs, ["en", "ja"], ["common"]).errors.join("\n"),
    /invalid placeholders/,
  );
});

test("calls, msg definitions and Trans literals require catalog entries", () => {
  for (const source of [
    't("Missing")',
    'translate("Missing")',
    'msg("Missing")',
    '<Trans message="Missing" />',
    '<Trans message={ok ? "Hello" : "Missing"} />',
  ]) {
    assert.match(
      sourceErrors(source).join("\n"),
      /missing source message "Missing"/,
      source,
    );
  }
  assert.deepEqual(
    sourceErrors(
      'const label = msg("Hello"); const ui = <Trans message="Hello" />; t(ok ? "Ready" : "Hello");',
    ),
    [],
  );
});

test("unmarked dynamic messages cannot silently escape coverage", () => {
  for (const source of [
    "t(label)",
    "translate(meta.label)",
    "<Trans message={label} />",
    "msg(prefix + suffix)",
  ]) {
    assert.match(
      sourceErrors(source).join("\n"),
      /dynamic source message must come from msg/,
      source,
    );
  }
});

test("SourceMessage brand requires every possible dynamic value to be marked", () => {
  const file = "/virtual/messages.ts";
  const source = `declare const sourceMessage: unique symbol;
    type SourceMessage = string & { readonly [sourceMessage]: true };
    declare const marked: SourceMessage;
    declare const mixed: SourceMessage | string;
    declare const anything: any;
    declare function t(value: string): string;
    t(marked); t(mixed); t(anything);`;
  const host = ts.createCompilerHost({});
  const original = host.getSourceFile.bind(host);
  host.getSourceFile = (name, ...args) =>
    name === file
      ? ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true)
      : original(name, ...args);
  const program = ts.createProgram([file], { noLib: true }, host);
  const tree = program.getSourceFile(file);
  const checker = program.getTypeChecker();
  assert.equal(checkSource(tree, file, messages, checker).length, 2);
  assert.equal(
    isMarkedType(
      checker.getTypeAtLocation(tree.statements[2].declarationList.declarations[0].name),
    ),
    true,
  );
});

test("JSX text and human-facing attributes require translation but technical attributes do not", () => {
  for (const source of [
    "<button>Hello</button>",
    '<div>{"Hello"}</div>',
    '<input placeholder="Hello" />',
    '<img alt="Hello" />',
    '<div aria-label={ok ? "Hello" : "Ready"} />',
  ]) {
    assert.match(sourceErrors(source).join("\n"), /unlocalized/, source);
  }
  assert.deepEqual(
    sourceErrors(
      '<div id="technical" className="flex" data-testid="hello" role="button"><input type="text" value="server-id" />{t("Hello")}<span>→</span></div>',
    ),
    [],
  );
});

test("translated comparison values and element identifiers are rejected", () => {
  assert.match(
    sourceErrors('if (t("Hello") === value) {}').join("\n"),
    /comparison values/,
  );
  assert.match(sourceErrors('<div key={t("Hello")} />').join("\n"), /identifiers stable/);
});

test("Rust tr/trf calls require cataloged literals, including raw and escaped strings", () => {
  assert.deepEqual(
    checkRustSource(
      'i18n::tr("Hello"); trf(r#"Welcome {name}"#, args); tr("\\u{48}ello");',
      "native.rs",
      messages,
    ),
    [],
  );
  assert.match(
    checkRustSource('tr("Missing")', "native.rs", messages).join("\n"),
    /missing source message/,
  );
  for (const source of [
    "tr(label)",
    'trf(format!("Hello"), args)',
    'tr("Hello".to_string())',
  ])
    assert.match(
      checkRustSource(source, "native.rs", messages).join("\n"),
      /must be a string literal/,
    );
});

test("Rust comments, string contents and function declarations are not translation calls", () => {
  const source =
    '// tr("Missing")\n/* nested /* tr("Missing") */ comment */\nconst DEMO: &str = r###"tr("Missing")"###; fn tr(message: &str) {}';
  assert.deepEqual(checkRustSource(source, "native.rs", messages), []);
});
