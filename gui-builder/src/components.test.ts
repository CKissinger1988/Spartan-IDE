/**
 * Real tests for component discovery (task #278) -- every case runs
 * against real files written to a real temp directory and parsed by the
 * real `parserAdapter`, never a stubbed AST.
 */
import { strict as assert } from "node:assert";
import { test } from "node:test";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { discoverComponents, discoverComponentsInSource, relativeSpecifier } from "./components.js";

function fixture(files: Record<string, string>): string {
  const root = mkdtempSync(join(tmpdir(), "spartan-components-"));
  for (const [rel, content] of Object.entries(files)) {
    const full = join(root, rel);
    mkdirSync(join(full, ".."), { recursive: true });
    writeFileSync(full, content);
  }
  return root;
}

test("discovers a real default-exported function component", () => {
  const root = fixture({
    "Card.jsx": "export default function Card() {\n  return <div />;\n}\n",
  });
  const found = discoverComponents(root);
  assert.equal(found.length, 1);
  assert.equal(found[0].name, "Card");
  assert.equal(found[0].isDefault, true);
});

test("discovers real named exports: function, const arrow, and an export list", () => {
  const root = fixture({
    "ui.jsx": [
      "export function Button() { return <button />; }",
      "const Panel = () => <section />;",
      "export const Badge = () => <span />;",
      "export { Panel };",
      "",
    ].join("\n"),
  });
  const names = discoverComponents(root)
    .map((c) => c.name)
    .sort();
  assert.deepEqual(names, ["Badge", "Button", "Panel"]);
  assert.ok(discoverComponents(root).every((c) => c.isDefault === false));
});

test("skips lowercase exports -- React itself treats those as DOM tags, not components", () => {
  const root = fixture({
    "mixed.jsx": [
      "export const helper = () => 1;",
      "export function useThing() { return 2; }",
      "export const Card = () => <div />;",
      "",
    ].join("\n"),
  });
  const names = discoverComponents(root).map((c) => c.name);
  assert.deepEqual(names, ["Card"]);
});

test("skips an anonymous default export rather than inventing a name for it", () => {
  const root = fixture({ "anon.jsx": "export default () => <div />;\n" });
  assert.deepEqual(discoverComponents(root), []);
});

test("never walks node_modules or build output", () => {
  const root = fixture({
    "App.jsx": "export default function App() { return <div />; }\n",
    "node_modules/pkg/Lib.jsx": "export default function Lib() { return <div />; }\n",
    "dist/Built.jsx": "export default function Built() { return <div />; }\n",
  });
  const names = discoverComponents(root).map((c) => c.name);
  assert.deepEqual(names, ["App"]);
});

test("a file that fails to parse is skipped, not fatal to the whole scan", () => {
  const root = fixture({
    "Good.jsx": "export default function Good() { return <div />; }\n",
    "Broken.jsx": "export default function Broken( { return <<< ;\n",
  });
  const names = discoverComponents(root).map((c) => c.name);
  assert.deepEqual(names, ["Good"]);
});

test("importFrom is a real relative specifier, and null for the target file itself", () => {
  const root = fixture({
    "pages/Home.jsx": "export default function Home() { return <div />; }\n",
    "shared/Button.jsx": "export function Button() { return <button />; }\n",
  });
  const from = join(root, "pages", "Home.jsx");
  const found = discoverComponents(root, from);

  const home = found.find((c) => c.name === "Home");
  const button = found.find((c) => c.name === "Button");
  assert.equal(home?.importFrom, null, "a component in the target file needs no import");
  assert.equal(button?.importFrom, "../shared/Button");
});

test("relativeSpecifier strips the extension and keeps a leading ./ for a sibling", () => {
  assert.equal(relativeSpecifier("/p/a/Home.jsx", "/p/a/Card.jsx"), "./Card");
  assert.equal(relativeSpecifier("/p/a/Home.jsx", "/p/b/Card.tsx"), "../b/Card");
});

test("discovers exported components from an unsaved source buffer", () => {
  const file = "/workspace/project/src/Live.tsx";
  const components = discoverComponentsInSource("export const LiveCard = () => <div />;\nexport default LiveCard;", file, file);
  assert.deepEqual(components.map(({ name, isDefault, importFrom }) => ({ name, isDefault, importFrom })), [
    { name: "LiveCard", isDefault: false, importFrom: null },
    { name: "LiveCard", isDefault: true, importFrom: null },
  ]);
});

test("indexes real direct JSX usages and deprecation replacement metadata", () => {
  const root = fixture({
    "Button.jsx": "/** @deprecated Use PrimaryButton instead. */\nexport function Button() { return <button />; }\n",
    "PrimaryButton.jsx": "export function PrimaryButton() { return <button />; }\n",
    "App.jsx": "import { Button } from './Button';\nexport default function App() { return <main><Button /><Button /></main>; }\n",
  });
  const button = discoverComponents(root).find((component) => component.name === "Button" && component.file.endsWith("Button.jsx"));
  assert.equal(button?.deprecated, true);
  assert.equal(button?.replacement, "PrimaryButton");
  assert.equal(button?.usageCount, 2);
  assert.ok(button?.usageFiles?.some((file) => file.endsWith("App.jsx")));
});

test("usage indexing ignores dependency and build output", () => {
  const root = fixture({
    "Card.jsx": "export function Card() { return <div />; }\n",
    "App.jsx": "export default function App() { return <Card />; }\n",
    "node_modules/pkg/Uses.jsx": "export default function Uses() { return <Card />; }\n",
    "dist/Built.jsx": "export default function Built() { return <Card />; }\n",
  });
  const card = discoverComponents(root).find((component) => component.name === "Card");
  assert.equal(card?.usageCount, 1);
  assert.equal(card?.usageFiles?.length, 1);
});

test("infers safe public prop hints from TypeScript props and destructured defaults", () => {
  const root = fixture({
    "Button.tsx": [
      "interface ButtonProps {",
      "  variant?: \"primary\" | \"secondary\";",
      "  disabled: boolean;",
      "  label?: string;",
      "}",
      "export function Button({ variant = \"primary\", disabled, label }: ButtonProps) {",
      "  return <button disabled={disabled}>{label ?? variant}</button>;",
      "}",
      "",
    ].join("\n"),
  });
  const button = discoverComponents(root).find((component) => component.name === "Button");
  assert.deepEqual(button?.propHints, [
    { name: "variant", type: '"primary" | "secondary"', required: false, defaultValue: '"primary"' },
    { name: "disabled", type: "boolean", required: true },
    { name: "label", type: "string", required: false },
  ]);
});

test("infers props from an exported arrow component and a type alias", () => {
  const root = fixture({
    "Card.tsx": [
      "type CardProps = { title: string; tone?: \"quiet\" | \"loud\" };",
      "export const Card = ({ title, tone }: CardProps) => <article data-tone={tone}>{title}</article>;",
      "",
    ].join("\n"),
  });
  const card = discoverComponents(root).find((component) => component.name === "Card");
  assert.deepEqual(card?.propHints, [
    { name: "title", type: "string", required: true },
    { name: "tone", type: '"quiet" | "loud"', required: false },
  ]);
});

test("resolves a relative imported props interface from a real TypeScript file", () => {
  const root = fixture({
    "types.ts": [
      "export interface ButtonProps {",
      "  label: string;",
      "  tone?: \"quiet\" | \"loud\";",
      "}",
      "",
    ].join("\n"),
    "Button.tsx": [
      "import type { ButtonProps as Props } from \"./types\";",
      "export function Button({ label, tone }: Props) {",
      "  return <button data-tone={tone}>{label}</button>;",
      "}",
      "",
    ].join("\n"),
  });
  const button = discoverComponents(root).find((component) => component.name === "Button");
  assert.deepEqual(button?.propHints, [
    { name: "label", type: "string", required: true },
    { name: "tone", type: '"quiet" | "loud"', required: false },
  ]);
});
