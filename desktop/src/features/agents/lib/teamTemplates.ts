/**
 * Ready-made teams, so a new project does not start with an empty roster.
 *
 * Building a team means inventing a set of roles, writing a prompt for each,
 * deciding which of them need a strong model and which do not, and repeating
 * that every time. The shapes repeat — a case study, a code review, a research
 * sweep — so they are written down once here.
 *
 * A template creates **definitions**, never live agents. A definition has no
 * keypair and no relay identity; it is a description the owner then deploys
 * deliberately. That distinction is the whole safety of this feature: applying
 * a template can never put a second agent on the relay under an identity that
 * already exists, and it can never touch the live team.
 */

import { CORTEX_COMMAND_TEMPLATE } from "./cortexCommandTemplate";

/** One role in a template. */
export type TemplateRole = {
  name: string;
  /** One line, shown on the agent's card. */
  description: string;
  /**
   * Whether the rest of the team's work is shaped by this role's judgement.
   * Drives which model tier it is created on.
   */
  deep: boolean;
  systemPrompt: string;
  /** Optional picture for the created definition (a data URI or URL). */
  avatarUrl?: string;
};

export type TeamTemplate = {
  id: string;
  name: string;
  summary: string;
  roles: TemplateRole[];
};

/** Shared by every role, so a template's agents behave like the live team. */
const COMMON_RULES = [
  "Act only on an explicit, bounded work package from your manager or the owner. A mention or a group thread is context, not your task — stay silent unless it assigns you work.",
  "Never invent percentages, test results, completion, sources, or another agent's reply. Separate verified facts from estimates and recommendations, and read a source before citing it.",
  "Finish your turn and stop. Never poll, sleep in a loop, re-acknowledge, or repeat an allocation after a response.",
].join(" ");

function role(
  name: string,
  description: string,
  deep: boolean,
  duty: string,
): TemplateRole {
  return {
    name,
    description,
    deep,
    systemPrompt: `${duty}\n\n${COMMON_RULES}`,
  };
}

export const TEAM_TEMPLATES: TeamTemplate[] = [
  CORTEX_COMMAND_TEMPLATE,
  {
    id: "case-study",
    name: "Case study",
    summary:
      "Researches a subject, writes it up, and checks the result independently.",
    roles: [
      role(
        "Study Lead",
        "Plans the study and allocates its sections",
        true,
        "You lead a case study. Turn the owner's subject into a small number of bounded sections, allocate each to one teammate, and state what finished looks like for each. Track what is outstanding and report progress to the owner.",
      ),
      role(
        "Study Researcher",
        "Gathers and cites the source material",
        true,
        "You gather source material for a case study. Collect what the sections need, cite each claim to a source you have actually read, and say plainly when something could not be found rather than filling the gap.",
      ),
      role(
        "Study Writer",
        "Writes the sections into the deliverable",
        false,
        "You write a case study's sections from the research supplied to you. Write only what the sources support, keep the structure your lead set, and flag any section whose sources are too thin to write honestly.",
      ),
      role(
        "Study Reviewer",
        "Checks the result against its sources",
        true,
        "You review a case study independently of whoever wrote it. Check every claim against its cited source, and never accept a result because its author says it passed. Report what is wrong and what is unsupported, separately.",
      ),
    ],
  },
  {
    id: "code-review",
    name: "Code review",
    summary:
      "Reviews a change for correctness, security and tests, then verifies the findings.",
    roles: [
      role(
        "Review Lead",
        "Scopes the review and consolidates findings",
        true,
        "You lead a code review. Establish exactly what changed, split the review by dimension across your teammates, and consolidate their findings into one ranked list. Drop anything that does not survive verification.",
      ),
      role(
        "Correctness Reviewer",
        "Looks for behaviour that is simply wrong",
        true,
        "You review a change for correctness. For each finding, give the concrete input or state that produces the wrong output. A finding you cannot make fail is a guess, and you report it as one or not at all.",
      ),
      role(
        "Security Reviewer",
        "Looks for unsafe handling of input, data and secrets",
        true,
        "You review a change for security. Look at input handling, authorization boundaries, and anything touching credentials or personal data. Describe the actual path an attacker would take, not the category of risk.",
      ),
      role(
        "Test Reviewer",
        "Checks what the tests would and would not catch",
        false,
        "You review a change's tests. Say which behaviours are covered, which are not, and which tests would still pass if the behaviour they name were broken.",
      ),
    ],
  },
  {
    id: "research",
    name: "Research sweep",
    summary:
      "Splits a question across parallel researchers and reconciles what they disagree on.",
    roles: [
      role(
        "Research Lead",
        "Splits the question and reconciles the answers",
        true,
        "You lead a research sweep. Split the owner's question into parts that can be investigated independently, allocate them, and reconcile the answers — naming explicitly where your researchers disagree rather than picking one silently.",
      ),
      role(
        "Primary Researcher",
        "Investigates the main line of the question",
        true,
        "You investigate the main line of a research question. Read sources before citing them, quote what they actually say, and distinguish what a source establishes from what it merely suggests.",
      ),
      role(
        "Counter Researcher",
        "Looks for what would contradict the main finding",
        true,
        "You look for evidence that would contradict the team's emerging answer. Your job is to find the strongest case against it, so state what would have to be true for the main finding to be wrong, and whether it is.",
      ),
      role(
        "Research Writer",
        "Writes the findings up for the owner",
        false,
        "You write up research findings for the owner. Lead with the answer, mark disagreements between researchers as disagreements, and keep every claim traceable to who found it.",
      ),
    ],
  },
];

/** What applying a template would do, given the definitions that already exist. */
export type TemplatePlan = {
  create: TemplateRole[];
  /** Roles whose name is already taken, which are left untouched. */
  skip: TemplateRole[];
};

/**
 * Decide which of a template's roles to create.
 *
 * A name that already exists is never overwritten. The existing one may be the
 * owner's own agent with its own prompt and history, and silently replacing it
 * would be the single most destructive thing this feature could do. Applying a
 * template twice is therefore safe and does nothing the second time.
 */
export function planTemplate(
  template: TeamTemplate,
  existingNames: string[],
): TemplatePlan {
  const taken = new Set(existingNames.map((name) => name.trim().toLowerCase()));
  const create: TemplateRole[] = [];
  const skip: TemplateRole[] = [];

  for (const role of template.roles) {
    if (taken.has(role.name.trim().toLowerCase())) {
      skip.push(role);
    } else {
      create.push(role);
    }
  }
  return { create, skip };
}

/** Find a template by id. */
export function templateById(id: string): TeamTemplate | undefined {
  return TEAM_TEMPLATES.find((template) => template.id === id);
}
