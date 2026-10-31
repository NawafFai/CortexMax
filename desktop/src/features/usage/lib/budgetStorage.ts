import type {
  ChannelBudgets,
  RuntimeBudgets,
} from "@/features/usage/lib/usageWindow";

/**
 * Where the spend budgets live, and the only place that knows it.
 *
 * These are display scales, not spending controls: they set where a bar turns
 * amber and then red. Nothing in the backend reads them, so a budget never
 * stops a run — the governor is the thing that acts, and it acts on provider
 * rate-limit windows rather than on dollars.
 *
 * They are kept in `localStorage` because that is what they are: a per-machine
 * preference about how to colour a bar. It follows that a budget does not sync
 * between machines, and comes back empty in a private window or with site data
 * cleared — hence the defaults below and the try/catch on every access.
 *
 * The keys were previously repeated as literals in four components, which had
 * already produced one divergence: two readers disagreed about whether a
 * stored `0` meant "no budget" or "fall back to the default". It means no
 * budget — `budgetZone` gives a non-positive budget no zones, so a deliberate
 * zero turns the bar off instead of silently rescaling it to $50.
 */

const DAILY_KEY = "cortex-max.daily-budget-usd";
const RUNTIME_KEY = "cortex-max.runtime-budgets-usd";
const CHANNEL_KEY = "cortex-max.channel-budgets-usd";

/** Daily budget assumed until the owner sets one. */
export const DEFAULT_BUDGET_USD = 50;

/** Read a number map, dropping entries that are not usable amounts. */
function readAmountMap(key: string): Record<string, number> {
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return {};
    const out: Record<string, number> = {};
    for (const [entry, value] of Object.entries(
      parsed as Record<string, unknown>,
    )) {
      const amount = Number(value);
      if (Number.isFinite(amount) && amount >= 0) out[entry] = amount;
    }
    return out;
  } catch {
    // Unreadable storage and unparseable JSON are the same outcome here: no
    // stored preference, so fall back to the caller's default.
    return {};
  }
}

function writeJson(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Persisting a preference is a convenience, never a correctness
    // requirement — a write that fails must not break the view.
  }
}

export function readDailyBudget(): number {
  try {
    const raw = window.localStorage.getItem(DAILY_KEY);
    if (raw == null) return DEFAULT_BUDGET_USD;
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : DEFAULT_BUDGET_USD;
  } catch {
    return DEFAULT_BUDGET_USD;
  }
}

export function writeDailyBudget(value: number): void {
  try {
    window.localStorage.setItem(DAILY_KEY, String(value));
  } catch {
    // See writeJson.
  }
}

export function readRuntimeBudgets(): RuntimeBudgets {
  return readAmountMap(RUNTIME_KEY);
}

export function writeRuntimeBudgets(budgets: RuntimeBudgets): void {
  writeJson(RUNTIME_KEY, budgets);
}

export function readChannelBudgets(): ChannelBudgets {
  return readAmountMap(CHANNEL_KEY);
}

export function writeChannelBudgets(budgets: ChannelBudgets): void {
  writeJson(CHANNEL_KEY, budgets);
}
