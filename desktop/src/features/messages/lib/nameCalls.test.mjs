import assert from "node:assert/strict";
import test from "node:test";

import { calledAgentPubkeys, normalizeArabic } from "./nameCalls.ts";

const CORTEX = { pubkey: "c".repeat(64), name: "CORTEX" };
const PILOT = { pubkey: "p".repeat(64), name: "Pilot Project Manager" };
const AXEL = { pubkey: "a".repeat(64), name: "Axel Process Engineer" };
const SAGE = { pubkey: "s".repeat(64), name: "Sage Safety Engineer" };
const SIGNAL = {
  pubkey: "9".repeat(64),
  name: "Signal Instrumentation Engineer",
};
const FORGE = { pubkey: "f".repeat(64), name: "Forge Mechanical Engineer" };
const VAULT = { pubkey: "v".repeat(64), name: "Vault Storage Engineer" };
const TEAM = [CORTEX, PILOT, AXEL, SAGE, SIGNAL, FORGE, VAULT];

const called = (text, team = TEAM) => new Set(calledAgentPubkeys(text, team));

test("an Arabic name calls its agent from anywhere in the message", () => {
  assert.deepEqual(called("كورتكس شو صار"), new Set([CORTEX.pubkey]));
  assert.deepEqual(
    called("ابي رد بايلوت و اكسل"),
    new Set([PILOT.pubkey, AXEL.pubkey]),
  );
});

test("the owner's own spellings and attached letters still call", () => {
  assert.deepEqual(called("يا ساقا وش رايك"), new Set([SAGE.pubkey]));
  assert.deepEqual(
    called("اسال بايلوت وسقنل"),
    new Set([PILOT.pubkey, SIGNAL.pubkey]),
  );
  assert.deepEqual(called("ياكورتكس"), new Set([CORTEX.pubkey]));
});

test("hamza, diacritics and letter variants do not matter", () => {
  assert.deepEqual(called("أكسل تعال"), new Set([AXEL.pubkey]));
  assert.deepEqual(called("كُورْتِكس"), new Set([CORTEX.pubkey]));
  assert.equal(normalizeArabic("إسلامٌ"), "اسلام");
});

test("an English name calls from where a name is used to address someone", () => {
  assert.deepEqual(called("Pilot, status?"), new Set([PILOT.pubkey]));
  assert.deepEqual(called("pilot شو الوضع"), new Set([PILOT.pubkey]));
  assert.deepEqual(called("hey Forge!"), new Set([FORGE.pubkey]));
  assert.deepEqual(called("please check @axel"), new Set([AXEL.pubkey]));
  assert.deepEqual(called("تمام يا Forge"), new Set([FORGE.pubkey]));
  assert.deepEqual(called("done.\nCortex: next?"), new Set([CORTEX.pubkey]));
});

test("an English name used as an ordinary word calls nobody", () => {
  assert.equal(called("Pilot plant sizing is done").size, 0);
  assert.equal(called("the pilot plant needs a signal check").size, 0);
  assert.equal(called("see #pilot-plant-sizing").size, 0);
});

test("an Arabic name that is also an everyday word needs an addressing position", () => {
  assert.equal(called("الجهد ٢٢٠ فولت").size, 0);
  assert.deepEqual(called("فولت، شو رايك"), new Set([VAULT.pubkey]));
  assert.deepEqual(called("يا فولت"), new Set([VAULT.pubkey]));
});

test("a full display name calls from anywhere", () => {
  assert.deepEqual(
    called("what does Signal Instrumentation Engineer think"),
    new Set([SIGNAL.pubkey]),
  );
});

test("only agents who can hear the message are called", () => {
  assert.equal(called("كورتكس", [PILOT]).size, 0);
});

test("a name two agents share calls neither", () => {
  const twin = { pubkey: "d".repeat(64), name: "Cortex Backup" };
  assert.equal(called("كورتكس", [CORTEX, twin]).size, 0);
});

test("an ion is not Orion", () => {
  const orion = { pubkey: "o".repeat(64), name: "Orion Research Scientist" };
  assert.equal(called("تركيز الأيون عالي", [orion]).size, 0);
  assert.deepEqual(called("اوريون", [orion]), new Set([orion.pubkey]));
});

test("@الكل calls every agent in the room", () => {
  const room = [
    { pubkey: "a", name: "CORTEX" },
    { pubkey: "b", name: "Nexus Backend Engineer" },
    { pubkey: "c", name: "Prism Software Architect" },
  ];
  for (const text of [
    "@الكل أنا سارة، أأذن لكم بالكتابة داخل مجلد المشروع",
    "أنا سارة @الكل ابدأوا",
    "@الجميع وقفوا شوي",
    "@all go ahead.",
    "Team, @everyone: stand by",
  ]) {
    assert.deepEqual(
      calledAgentPubkeys(text, room).sort(),
      ["a", "b", "c"],
      text,
    );
  }
});

test("the everyday word, or an address that merely contains it, calls nobody", () => {
  const room = [{ pubkey: "a", name: "CORTEX" }];
  for (const text of [
    "الكل يعرف هذا",
    "write to ops@all.example.com",
    "@allergy notes",
  ]) {
    assert.deepEqual(calledAgentPubkeys(text, room), [], text);
  }
});
