/**
 * The note a team leaves for itself.
 *
 * Agents coordinate well inside one conversation and badly across turns: an
 * ACP session re-sends its history, but a different agent's session never saw
 * it, and a restarted one has forgotten. So the thing they actually share is
 * a file — and this project already proved the point, where the useful
 * coordination turned out to live in the repository's own HANDOVER.md rather
 * than in anything the app held.
 *
 * The template is deliberately small. A long form gets filled in once and
 * abandoned; these are the four questions a colleague picking the work up
 * actually needs answered, and nothing else.
 */

export const HANDOVER_FILENAME = "HANDOVER.md";

export function handoverTemplate(channelName: string, isoDate: string): string {
  return `# Handover — ${channelName}

Last updated: ${isoDate}

This file is how this team hands work between each other. Anyone picking
something up reads it first; anyone putting something down updates it before
they stop. Keep it short enough that people actually read it.

## State of play

What is true right now. Facts with evidence, not intentions.

- _(nothing recorded yet)_

## In flight

Work started and not finished, with who holds it. An item here that nobody
holds is the most expensive line in the file — say so rather than leave it.

| Item | Held by | Blocked on |
| --- | --- | --- |
| _(none)_ | | |

## Decisions the owner still holds

Things nobody below the reporting line may decide. Each with the date it was
raised, so an old one is visibly old.

- _(none)_

## Do not build on this yet

Work that looks finished but has no evidence behind it. This section exists
because an unverified increment that gets built upon costs more to unwind
than it did to write.

- _(none)_
`;
}
