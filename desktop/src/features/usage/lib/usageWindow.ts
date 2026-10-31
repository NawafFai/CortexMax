/**
 * Local-midnight bucket boundaries for the NIP-AM usage series.
 *
 * The Rust side validates that boundaries are strictly increasing and that no
 * interval exceeds 48 hours, and it deliberately refuses boundaries built as
 * `n * 86_400`: across a daylight-saving transition a civil day is 23 or 25
 * hours, so fixed-width arithmetic silently shifts every bucket after the
 * change. These helpers walk calendar days through `Date`, which applies the
 * host's own zone rules, so each bucket is exactly one civil day in the
 * owner's local time.
 */

/** Windows the dashboard offers. Values are bucket (day) counts. */
export const USAGE_WINDOWS = [7, 30] as const;
export type UsageWindowDays = (typeof USAGE_WINDOWS)[number];

/** Unix seconds at the start of the local calendar day containing `date`. */
export function localMidnightUnix(date: Date): number {
  const midnight = new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
    0,
    0,
    0,
    0,
  );
  return Math.floor(midnight.getTime() / 1000);
}

/**
 * Boundaries for the last `days` civil days ending with the day containing
 * `now`, inclusive. Returns `days + 1` entries: N boundaries bound N-1
 * buckets, so the last pair is today's midnight to tomorrow's midnight and
 * today's partial spend lands in the final bucket.
 */
export function buildBucketBoundaries(
  days: number,
  now: Date = new Date(),
): number[] {
  const count = Math.max(1, Math.floor(days));
  const boundaries: number[] = [];
  // Start `count` days back, then step forward one calendar day at a time so
  // the host's zone rules — not arithmetic — decide each boundary.
  const cursor = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() - count + 1,
    0,
    0,
    0,
    0,
  );
  for (let index = 0; index <= count; index += 1) {
    boundaries.push(localMidnightUnix(cursor));
    cursor.setDate(cursor.getDate() + 1);
  }
  return boundaries;
}

/**
 * Parse a `UsageField`/`CostField`-style numeric string.
 *
 * Token counts cross the wire as strings because they can exceed
 * `Number.MAX_SAFE_INTEGER` in aggregate; the dashboard only ever renders
 * them, so a lossy read is acceptable here, but an unparseable or absent
 * value must stay `null` rather than collapsing to 0 — unknown is not zero.
 */
export function readTokenField(
  field: { value: string | null } | null | undefined,
): number | null {
  if (!field || field.value == null) return null;
  const parsed = Number(field.value);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Budget zones, in ascending severity. */
export type BudgetZone = "ok" | "warn" | "over";

/** Fraction of the budget at which the bar turns yellow, then red. */
export const BUDGET_WARN_AT = 0.7;
export const BUDGET_OVER_AT = 0.9;

/**
 * A provider window is read like a battery: what is left, not what is spent.
 * Full is green; from half gone it turns yellow; with this little left it is
 * red, because the run is about to stop. Spend against the owner's own budget
 * keeps its spent-so-far reading ({@link budgetZone}).
 */
export const LIMIT_WARN_REMAINING = 50;
export const LIMIT_OVER_REMAINING = 15;

/** Percent of a window still left, 0-100, from the percent a provider says is used. */
export function remainingPercent(usedPercent: number): number {
  return Math.max(0, Math.min(100, Math.round(100 - usedPercent)));
}

/** The colour zone for a provider window, judged by what is left of it. */
export function limitZone(usedPercent: number): BudgetZone {
  const left = remainingPercent(usedPercent);
  if (left <= LIMIT_OVER_REMAINING) return "over";
  if (left <= LIMIT_WARN_REMAINING) return "warn";
  return "ok";
}

/**
 * Classify spend against a daily budget. A non-positive or absent budget has
 * no zones — the caller shows the raw figure instead of a misleading bar.
 */
export function budgetZone(
  spend: number | null,
  budget: number | null,
): BudgetZone | null {
  if (spend == null || budget == null || budget <= 0) return null;
  const fraction = spend / budget;
  if (fraction >= BUDGET_OVER_AT) return "over";
  if (fraction >= BUDGET_WARN_AT) return "warn";
  return "ok";
}

/** Bar fill as a 0-1 fraction, clamped so an overrun still renders a full bar. */
export function budgetFraction(
  spend: number | null,
  budget: number | null,
): number {
  if (spend == null || budget == null || budget <= 0) return 0;
  return Math.max(0, Math.min(1, spend / budget));
}

/** Compact token count for a dense table: 1234567 -> "1.2M". */
export function formatTokens(value: number | null): string {
  if (value == null) return "—";
  if (value >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(Math.round(value));
}

/** USD with cents, or an em dash when no turn in scope reported a price. */
export function formatUsd(value: number | null): string {
  if (value == null) return "—";
  return `$${value.toFixed(2)}`;
}

// ── Per-runtime rollup ──────────────────────────────────────────────────────

/**
 * What a runtime's numbers are actually worth.
 *
 * Harnesses differ in what they report, and the difference matters more than
 * the totals: `claude-agent-acp` sends tokens and a cost, `codex-acp` sends
 * tokens but never a price, and a harness that reports nothing leaves no rows
 * at all. Collapsing those three cases into one "$0.00" would read as free
 * when it means unmeasured, so each runtime carries its own confidence.
 */
export type RuntimeCoverage = "full" | "tokens-only" | "unreported";

export type RuntimeUsageRow = {
  /** Harness id as reported, e.g. `claude-agent-acp`. */
  harness: string;
  /** Short name for the UI. */
  label: string;
  turns: number;
  inputTokens: number | null;
  outputTokens: number | null;
  /** `null` when this runtime never reports a price. */
  costUsd: number | null;
  coverage: RuntimeCoverage;
};

const RUNTIME_LABELS: Record<string, string> = {
  "claude-agent-acp": "Claude",
  "claude-code-acp": "Claude",
  "codex-acp": "Codex",
  kimi: "Kimi",
  "kimi-code": "Kimi",
  opencode: "OpenCode",
  "buzz-agent": "Buzz Agent",
  goose: "Goose",
};

/** Friendly name for a harness id, falling back to the id itself. */
export function runtimeLabel(harness: string): string {
  return RUNTIME_LABELS[harness] ?? harness;
}

type UsageLike = {
  inputTokens: { value: string | null };
  outputTokens: { value: string | null };
  estimatedCostUsd: { value: number | null };
};

type ModelLike = {
  harness: string | null;
  usage: UsageLike;
  reportCount: number;
};

type AgentLike = { models: ModelLike[] };

/**
 * Roll a usage series up by harness.
 *
 * Reads each agent's per-model rows, which are the only place the wire format
 * carries the harness. A model row with no harness is folded into `unknown`
 * rather than dropped, so the runtime totals always re-add to the series
 * total and a gap is visible instead of silently missing.
 */
export function rollupByRuntime(agents: AgentLike[]): RuntimeUsageRow[] {
  const groups = new Map<
    string,
    {
      turns: number;
      input: number | null;
      output: number | null;
      cost: number | null;
      sawCost: boolean;
    }
  >();

  for (const agent of agents) {
    for (const model of agent.models ?? []) {
      const harness = model.harness ?? "unknown";
      const entry = groups.get(harness) ?? {
        turns: 0,
        input: null,
        output: null,
        cost: null,
        sawCost: false,
      };
      entry.turns += model.reportCount ?? 0;
      const input = readTokenField(model.usage?.inputTokens);
      if (input != null) entry.input = (entry.input ?? 0) + input;
      const output = readTokenField(model.usage?.outputTokens);
      if (output != null) entry.output = (entry.output ?? 0) + output;
      const cost = model.usage?.estimatedCostUsd?.value ?? null;
      if (cost != null) {
        entry.cost = (entry.cost ?? 0) + cost;
        entry.sawCost = true;
      }
      groups.set(harness, entry);
    }
  }

  const rows: RuntimeUsageRow[] = [];
  for (const [harness, entry] of groups) {
    const coverage: RuntimeCoverage = entry.sawCost
      ? "full"
      : entry.turns > 0
        ? "tokens-only"
        : "unreported";
    rows.push({
      harness,
      label: runtimeLabel(harness),
      turns: entry.turns,
      inputTokens: entry.input,
      outputTokens: entry.output,
      costUsd: entry.sawCost ? entry.cost : null,
      coverage,
    });
  }
  rows.sort((left, right) => {
    const l = left.costUsd ?? -1;
    const r = right.costUsd ?? -1;
    if (r !== l) return r - l;
    return (right.inputTokens ?? 0) - (left.inputTokens ?? 0);
  });
  return rows;
}

/** A model's measured record over the window. */
export type ModelUsageRow = {
  /** `harness::model`, unique across runtimes that share a model name. */
  key: string;
  harness: string;
  runtimeLabel: string;
  model: string;
  agents: number;
  turns: number;
  inputTokens: number | null;
  outputTokens: number | null;
  costUsd: number | null;
  /** Cost divided by turns, or `null` when either is missing. */
  costPerTurn: number | null;
  /** Output tokens per turn — how much this model writes for a turn's work. */
  outputPerTurn: number | null;
  /** False when the row has too few turns to rank against the others. */
  comparable: boolean;
};

/**
 * Turns below which a model's average is noise rather than a measurement.
 *
 * One expensive turn is not a verdict on a model. Ranking on a handful of
 * turns produces a table that reorders itself every hour and invites a
 * decision the data does not support, which is worse than saying "not yet".
 */
export const MIN_TURNS_TO_COMPARE = 20;

type ModelRowLike = ModelLike & { model: string | null };
type AgentModelsLike = { agentPubkey?: string; models: ModelRowLike[] };

/**
 * Roll a usage series up by model, for comparing models against each other.
 *
 * Cost per turn is the figure worth having: a model with a high total may
 * simply have done more work, and only the per-turn rate says which model is
 * expensive. Output tokens per turn sits beside it because the two together
 * separate "costs more per token" from "writes more per turn" — different
 * problems with different fixes.
 *
 * What this deliberately does NOT report is a failure rate per model, though
 * that would be the other half of a comparison. A turn that fails to parse is
 * stored with its model column empty (see the invalid-row constructor in
 * `archive/metric_store.rs`), so the archive cannot say which model produced
 * it. The failure counts that do exist are per agent, and attributing an
 * agent's past failures to the model it runs today would be wrong in exactly
 * the case that matters — a team retiered between models mid-window.
 */
export function rollupByModel(agents: AgentModelsLike[]): ModelUsageRow[] {
  const groups = new Map<
    string,
    {
      harness: string;
      model: string;
      agents: Set<string>;
      turns: number;
      input: number | null;
      output: number | null;
      cost: number | null;
      sawCost: boolean;
    }
  >();

  for (const [index, agent] of agents.entries()) {
    for (const row of agent.models ?? []) {
      // A row with no model cannot be compared against one that names its
      // model, so it groups as unknown rather than merging into a named model
      // and quietly skewing that model's average.
      const harness = row.harness ?? "unknown";
      const model = row.model ?? "unknown";
      const key = `${harness}::${model}`;
      const entry = groups.get(key) ?? {
        harness,
        model,
        agents: new Set<string>(),
        turns: 0,
        input: null,
        output: null,
        cost: null,
        sawCost: false,
      };
      entry.agents.add(agent.agentPubkey ?? `#${index}`);
      entry.turns += row.reportCount ?? 0;
      const input = readTokenField(row.usage?.inputTokens);
      if (input != null) entry.input = (entry.input ?? 0) + input;
      const output = readTokenField(row.usage?.outputTokens);
      if (output != null) entry.output = (entry.output ?? 0) + output;
      const cost = row.usage?.estimatedCostUsd?.value ?? null;
      if (cost != null) {
        entry.cost = (entry.cost ?? 0) + cost;
        entry.sawCost = true;
      }
      groups.set(key, entry);
    }
  }

  const rows: ModelUsageRow[] = [];
  for (const [key, entry] of groups) {
    const costUsd = entry.sawCost ? entry.cost : null;
    rows.push({
      key,
      harness: entry.harness,
      runtimeLabel: runtimeLabel(entry.harness),
      model: entry.model,
      agents: entry.agents.size,
      turns: entry.turns,
      inputTokens: entry.input,
      outputTokens: entry.output,
      costUsd,
      costPerTurn:
        costUsd != null && entry.turns > 0 ? costUsd / entry.turns : null,
      outputPerTurn:
        entry.output != null && entry.turns > 0
          ? entry.output / entry.turns
          : null,
      comparable: entry.turns >= MIN_TURNS_TO_COMPARE,
    });
  }

  // Comparable rows first, dearest-per-turn at the top, since the question
  // this answers is "what is costing me most for each turn it does". Rows
  // below the floor sort last whatever their average, so a two-turn fluke
  // cannot take the headline position.
  rows.sort((left, right) => {
    if (left.comparable !== right.comparable) return left.comparable ? -1 : 1;
    const l = left.costPerTurn ?? -1;
    const r = right.costPerTurn ?? -1;
    if (r !== l) return r - l;
    return right.turns - left.turns;
  });
  return rows;
}

/** One line explaining what a runtime's figures do and do not cover. */
export function coverageNote(coverage: RuntimeCoverage): string {
  switch (coverage) {
    case "full":
      return "tokens and cost reported";
    case "tokens-only":
      return "tokens only — this harness reports no price";
    default:
      return "no usage reported by this harness";
  }
}

/** Per-runtime budgets, keyed by harness id. */
export type RuntimeBudgets = Record<string, number>;

/** Budget for a harness, or `null` when the owner has not set one. */
export function runtimeBudget(
  budgets: RuntimeBudgets,
  harness: string,
): number | null {
  const value = budgets[harness];
  return Number.isFinite(value) && value > 0 ? value : null;
}

/** Per-project budgets, keyed by the channel the project runs in. */
export type ChannelBudgets = Record<string, number>;

/**
 * Budget for one project, falling back to the account-wide daily figure.
 *
 * The fallback is why this takes one: measuring every channel against a single
 * daily budget makes each bar answer a question nobody asked — with eight
 * channels sharing $50, all eight read green until one of them has spent the
 * whole day. A project's own budget makes its bar mean "this project", and
 * channels without one keep the previous behaviour rather than losing a scale.
 *
 * A deliberate `0` is a budget, not a missing one: it suppresses the bar,
 * which is the honest rendering for work the owner does not want scaled.
 */
export function channelBudget(
  budgets: ChannelBudgets,
  channelId: string,
  fallback: number,
): number {
  const value = budgets[channelId];
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

/**
 * How long until a reset, written the way a person reads a deadline.
 *
 * The unit shown follows how much time is left, because the precision that
 * matters changes with it: days out, seconds are noise; minutes out, they are
 * the whole point. A reset already past reads as due rather than as a negative
 * countdown, since a window that has rolled over is not "-3 minutes away".
 */
export function formatCountdown(secondsRemaining: number): string {
  if (!Number.isFinite(secondsRemaining) || secondsRemaining <= 0) {
    return "due now";
  }
  const total = Math.floor(secondsRemaining);
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3_600);
  const minutes = Math.floor((total % 3_600) / 60);
  const seconds = total % 60;

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  // Under an hour the seconds are what the owner is actually watching.
  return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
}

/**
 * A reset instant the way the provider's own usage page states it — "Wed, Sep
 * 30, 2:15 PM" — so a bar answers "when" as well as "how long".
 *
 * `timeZone` exists for tests; the app shows the machine's own zone, which is
 * the one the owner reads a clock in.
 */
export function formatResetInstant(
  epochSeconds: number,
  timeZone?: string,
): string {
  return new Date(epochSeconds * 1000).toLocaleString("en-US", {
    weekday: "short",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZone,
  });
}

/** What a window's current burn rate implies, if anything. */
export type Forecast = {
  /** Unix seconds the window is projected to be spent. */
  at: number;
  /** Seconds from now until that moment. */
  inSeconds: number;
};

/**
 * Minimum share of a window that must have elapsed before its rate is worth
 * extrapolating.
 *
 * Ten minutes into a weekly window, a single burst of work implies the whole
 * week will be gone by lunchtime. That projection is arithmetically correct
 * and practically nonsense, and a warning that cries wolf on the first busy
 * hour is one the owner learns to ignore — which costs more than showing
 * nothing.
 */
const MIN_ELAPSED_FRACTION = 0.1;

/**
 * When a window will be spent at its current rate, or null when the data does
 * not support saying.
 *
 * Returns null — rather than a guess — when the window length is unknown, when
 * too little of it has elapsed to extrapolate, when nothing has been spent,
 * and when the projection lands after the reset. That last case is the normal
 * one: a window that will not run out does not need a forecast, and drawing
 * one anyway turns a calm week into a warning.
 */
export function forecastExhaustion({
  usedPercent,
  resetsAt,
  windowMinutes,
  now,
}: {
  usedPercent: number;
  resetsAt: number | null;
  windowMinutes: number | null;
  now: number;
}): Forecast | null {
  if (resetsAt == null || windowMinutes == null || windowMinutes <= 0) {
    return null;
  }
  const windowSeconds = windowMinutes * 60;
  const startedAt = resetsAt - windowSeconds;
  const elapsed = now - startedAt;
  if (elapsed <= 0 || now >= resetsAt) return null;

  const elapsedFraction = elapsed / windowSeconds;
  if (elapsedFraction < MIN_ELAPSED_FRACTION) return null;

  const usedFraction = usedPercent / 100;
  if (usedFraction <= 0) return null;
  // Already spent: there is nothing left to forecast.
  if (usedFraction >= 1) return null;

  // At this rate, the whole window is spent once elapsed reaches
  // elapsed / used — the same burn carried forward.
  const at = startedAt + elapsed / usedFraction;
  if (at >= resetsAt) return null;

  return { at, inSeconds: at - now };
}
