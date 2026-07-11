const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { loadParser } = require("./parser");

test("real Rust grammar parses valid source with zero errors", async () => {
  const { parser } = await loadParser("rust");
  const source = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
  const tree = parser.parse(source);
  assert.equal(tree.rootNode.type, "source_file");
  assert.equal(tree.rootNode.hasError(), false);
});

test("real Rust grammar reports a genuine syntax error on invalid source", async () => {
  const { parser } = await loadParser("rust");
  const tree = parser.parse("fn add(a: i32, b {{{ not valid rust");
  assert.equal(tree.rootNode.hasError(), true);
});

test("real Rust grammar resolves a function's real name via a field lookup", async () => {
  const { parser } = await loadParser("rust");
  const tree = parser.parse("fn my_function_name() {}\n");
  const fnNode = tree.rootNode.descendantsOfType("function_item")[0];
  const nameNode = fnNode.childForFieldName("name");
  assert.equal(nameNode.text, "my_function_name");
});

test("a real query against real Rust source captures comment/string/function/number nodes", async () => {
  const { parser, language } = await loadParser("rust");
  const source = fs.readFileSync(path.join(__dirname, "..", "fixtures", "sample.rs"), "utf8");
  const tree = parser.parse(source);
  const queryText = fs.readFileSync(path.join(__dirname, "..", "queries", "rust.scm"), "utf8");
  const query = language.query(queryText);
  const captures = query.captures(tree.rootNode);

  const byName = (name) => captures.filter((c) => c.name === name);
  assert.ok(byName("comment").length >= 1, "expected at least one real comment capture");
  assert.ok(byName("string").length >= 1, "expected at least one real string capture");
  assert.ok(byName("function").length >= 1, "expected at least one real function-name capture");
  assert.ok(byName("number").length >= 1, "expected at least one real number capture");

  const fnCapture = byName("function")[0];
  assert.equal(fnCapture.node.text, "add", "the captured function name must match the real fixture");
});

test("real Python grammar parses valid source and resolves a function's real name", async () => {
  const { parser } = await loadParser("python");
  const tree = parser.parse('def greet(name):\n    return "hello " + name\n');
  assert.equal(tree.rootNode.hasError(), false);
  const fnNode = tree.rootNode.descendantsOfType("function_definition")[0];
  assert.equal(fnNode.childForFieldName("name").text, "greet");
});

test("a real query against real Python source captures comment/string/function nodes", async () => {
  const { parser, language } = await loadParser("python");
  const source = fs.readFileSync(path.join(__dirname, "..", "fixtures", "sample.py"), "utf8");
  const tree = parser.parse(source);
  const queryText = fs.readFileSync(path.join(__dirname, "..", "queries", "python.scm"), "utf8");
  const query = language.query(queryText);
  const captures = query.captures(tree.rootNode);

  const byName = (name) => captures.filter((c) => c.name === name);
  assert.ok(byName("comment").length >= 1);
  assert.ok(byName("string").length >= 1);
  assert.equal(byName("function")[0].node.text, "greet");
});
