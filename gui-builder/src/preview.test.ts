import test from "node:test";
import assert from "node:assert/strict";
import { buildPreviewDocument } from "./preview.js";

test("builds a standalone preview document with the responsive viewport metadata", () => {
  const document = buildPreviewDocument("console.log('preview')", "Card & preview");
  assert.match(document, /<meta name="viewport" content="width=device-width, initial-scale=1">/);
  assert.match(document, /<title>Card &amp; preview<\/title>/);
  assert.match(document, /<div id="spartan-root"><\/div>/);
  assert.match(document, /console\.log\('preview'\)/);
});

test("escapes literal closing script markers inside bundled JavaScript", () => {
  const document = buildPreviewDocument("const html = '</script><div>safe</div>'; ");
  assert.match(document, /<script>const html = '<\\\/script><div>safe<\/div>'; <\/script>/);
  assert.doesNotMatch(document, /<script>const html = '<\/script>/);
});

test("escapes HTML-sensitive title characters", () => {
  const document = buildPreviewDocument("", '<Preview> "ready"');
  assert.match(document, /<title>&lt;Preview&gt; &quot;ready&quot;<\/title>/);
});
