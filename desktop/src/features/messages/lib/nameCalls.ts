/**
 * Agents the owner called by name, without an @.
 *
 * In a chat you say someone's name to talk to them. The composer's @ picker
 * is the precise way; this is the natural one: "كورتكس شو صار؟" or
 * "Pilot, status?" reaches CORTEX or Pilot exactly as a mention would — the
 * message goes out with their `p` tag, so they wake and answer.
 *
 * Only the owner's own sends are read this way. Agents address each other
 * with explicit mentions, which is what keeps a room of agents from waking
 * one another in a loop.
 *
 * Names are matched with care, because many of them are ordinary words: a
 * pilot plant, a signal, a scope of work, 220 volts. Arabic spellings are
 * transliterations nobody writes by accident, so they call from anywhere in
 * the message — except the few that are also everyday words (فولت, بايت,
 * بروب…), which call only from where a name is used to address someone: the
 * start of a line, after "يا", or after an @. English names are all treated
 * that way, and at the start they must not run straight into another English
 * word — "Pilot, status?" calls Pilot; "Pilot plant sizing is done" does not.
 * A full display name ("Signal Instrumentation Engineer") calls from anywhere.
 */

/** One of the owner's agents who can hear this message. */
export type NameCallCandidate = {
  pubkey: string;
  /** Display name, e.g. "Pilot Project Manager" or "CORTEX". */
  name: string;
};

/**
 * Arabic spellings of each agent's first name, keyed by the lowercased
 * English first name. Includes colloquial spellings seen in use ("ساقا" for
 * Sage, "سقنل" for Signal). Compared after {@link normalizeArabic}, so
 * hamza, alef and ta-marbuta variants need no entries of their own.
 */
const ARABIC_NAMES: Readonly<Record<string, readonly string[]>> = {
  aegis: ["ايجس", "ايجيس"],
  atlas: ["اطلس"],
  axel: ["اكسل", "اكسيل"],
  byte: ["بايت"],
  comply: ["كومبلاي", "كمبلاي"],
  cortex: ["كورتكس", "كورتيكس"],
  forge: ["فورج", "فورق"],
  judge: ["جادج", "جدج"],
  kernel: ["كيرنل", "كرنل"],
  nexus: ["نيكسس", "نكسس", "نيكسوس"],
  nova: ["نوفا"],
  operator: ["اوبريتر", "اوبريتور", "اوبرايتر"],
  orion: ["اوريون", "اوريان"],
  pilot: ["بايلوت", "بايلت"],
  pixel: ["بكسل", "بيكسل"],
  prism: ["بريزم", "بريسم"],
  probe: ["بروب"],
  quill: ["كويل"],
  relay: ["ريلاي", "ريلي"],
  sage: ["سيج", "ساقا", "سايج", "ساغا"],
  scope: ["سكوب"],
  scribe: ["سكرايب", "سكريب"],
  signal: ["سيجنال", "سقنل", "سيقنل", "سغنل", "سجنال", "سيغنال", "سقنال"],
  syntax: ["سينتاكس", "سنتاكس"],
  terra: ["تيرا"],
  value: ["فاليو"],
  vault: ["فولت"],
  vector: ["فكتور", "فيكتور"],
};

/**
 * Arabic spellings that are also everyday words — a unit, a device, a term
 * of the trade. These call only from an addressing position.
 */
const ORDINARY_ARABIC = new Set(
  [
    "بايت",
    "بكسل",
    "بيكسل",
    "اطلس",
    "فولت",
    "ريلاي",
    "ريلي",
    "بروب",
    "كيرنل",
    "كرنل",
    "فكتور",
    "فيكتور",
    "تيرا",
  ].map(normalizeArabic),
);

/** Single letters that attach to the front of an Arabic word: and, so, to, with. */
const ATTACHED_PREFIXES = ["ويا", "يا", "و", "ف", "ل", "ب"];

/** Words that may open a message before the name being called. */
const GREETINGS = new Set(["hey", "hi", "hello", "dear", "يا", "هاي", "هلا"]);

const LATIN_WORD = /^[a-z][a-z0-9'_-]*$/;
const TOKEN = /[\p{L}\p{N}@'_-]+/gu;
const CALL_PUNCTUATION = /^\s*[,:!?؟،؛.]/;

/**
 * Fold the spellings of one Arabic word together: drop diacritics and
 * tatweel, and unify the alef, ya and ta-marbuta forms people type
 * interchangeably. Latin text is lowercased.
 */
export function normalizeArabic(text: string): string {
  return text
    .replace(/[\u064B-\u065F\u0670\u0640]/g, "")
    .replace(/[أإآٱ]/g, "ا")
    .replace(/ى/g, "ي")
    .replace(/ة/g, "ه")
    .replace(/ؤ/g, "و")
    .replace(/ئ/g, "ي")
    .toLowerCase();
}

function firstName(name: string): string {
  const first = name.trim().split(/\s+/)[0] ?? "";
  return first.toLowerCase().replace(/[^a-z0-9]/g, "");
}

type Token = { word: string; start: number; end: number };

/**
 * "@الكل" — or "@الجميع", "@all", "@everyone" — calls every agent in the
 * room. An owner writing to the whole project room otherwise reached no one:
 * a room hears only the agents it names, and the message that authorised the
 * team to proceed sat unread while the team waited for it. Only with the @,
 * so the everyday "الكل" in a sentence calls nobody.
 */
const WHOLE_ROOM = /(^|[\s(])@(الكل|الجميع|all|everyone)(?=$|[\s.,،!?:؛)])/iu;

/** Whether this message calls every agent in the room. */
export function callsWholeRoom(text: string): boolean {
  return WHOLE_ROOM.test(text);
}

/**
 * Pubkeys of the candidates this message calls by name, in no particular
 * order. A name two candidates share calls neither: a guess is worse than
 * asking for an @.
 */
export function calledAgentPubkeys(
  text: string,
  candidates: readonly NameCallCandidate[],
): string[] {
  if (callsWholeRoom(text)) {
    return candidates.map((candidate) => candidate.pubkey);
  }
  const normalized = normalizeArabic(text);
  const tokens: Token[] = [];
  for (const match of normalized.matchAll(TOKEN)) {
    const start = match.index ?? 0;
    tokens.push({ word: match[0], start, end: start + match[0].length });
  }
  if (tokens.length === 0) return [];

  const byArabic = new Map<string, NameCallCandidate[]>();
  const byEnglish = new Map<string, NameCallCandidate[]>();
  const add = (
    index: Map<string, NameCallCandidate[]>,
    key: string,
    candidate: NameCallCandidate,
  ) => {
    if (!key) return;
    const existing = index.get(key) ?? [];
    if (!existing.some((entry) => entry.pubkey === candidate.pubkey)) {
      existing.push(candidate);
    }
    index.set(key, existing);
  };
  for (const candidate of candidates) {
    const english = firstName(candidate.name);
    add(byEnglish, english, candidate);
    for (const spelling of ARABIC_NAMES[english] ?? []) {
      add(byArabic, normalizeArabic(spelling), candidate);
    }
  }

  const called = new Set<string>();
  const call = (matches: NameCallCandidate[] | undefined) => {
    if (matches?.length === 1) called.add(matches[0].pubkey);
  };

  // The first word of the message or of one of its lines…
  const startsLine = (index: number) =>
    index === 0 ||
    normalized.slice(tokens[index - 1].end, tokens[index].start).includes("\n");
  // …or the word right after a greeting that opens one.
  const opensLine = (index: number) =>
    startsLine(index) ||
    (index > 0 &&
      GREETINGS.has(tokens[index - 1].word) &&
      startsLine(index - 1));

  for (const [index, token] of tokens.entries()) {
    const at = token.word.startsWith("@");
    const word = at ? token.word.slice(1) : token.word;
    const afterYa = index > 0 && tokens[index - 1].word === "يا";
    const addressing = at || afterYa || opensLine(index);

    if (LATIN_WORD.test(word)) {
      const matches = byEnglish.get(word);
      if (!matches) continue;
      if (at || afterYa) {
        call(matches);
        continue;
      }
      if (!opensLine(index)) continue;
      const next = tokens[index + 1];
      const gap = normalized.slice(token.end, next?.start ?? normalized.length);
      if (!next || CALL_PUNCTUATION.test(gap) || !LATIN_WORD.test(next.word)) {
        call(matches);
      }
      continue;
    }

    for (const prefix of ["", ...ATTACHED_PREFIXES]) {
      if (prefix && !word.startsWith(prefix)) continue;
      const stem = word.slice(prefix.length);
      const matches = byArabic.get(stem);
      if (!matches) continue;
      const vocative = prefix === "يا" || prefix === "ويا";
      if (!ORDINARY_ARABIC.has(stem) || addressing || vocative) call(matches);
      break;
    }
  }

  // A full display name is unambiguous wherever it appears.
  for (const candidate of candidates) {
    const full = normalizeArabic(candidate.name.trim());
    if (!full.includes(" ")) continue;
    const at = normalized.indexOf(full);
    if (at < 0) continue;
    const before = normalized[at - 1];
    const after = normalized[at + full.length];
    const boundary = (char: string | undefined) =>
      char === undefined || !/[\p{L}\p{N}]/u.test(char);
    if (boundary(before) && boundary(after)) called.add(candidate.pubkey);
  }

  return [...called];
}
