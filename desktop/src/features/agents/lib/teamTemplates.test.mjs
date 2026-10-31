import assert from "node:assert/strict";
import { test } from "node:test";

import { TEAM_TEMPLATES, planTemplate, templateById } from "./teamTemplates.ts";

const caseStudy = templateById("case-study");

test("every template has an id, a name and at least two roles", () => {
  assert.ok(TEAM_TEMPLATES.length > 0);
  for (const template of TEAM_TEMPLATES) {
    assert.ok(template.id, "a template needs an id");
    assert.ok(template.name, `${template.id} needs a name`);
    assert.ok(template.summary, `${template.id} needs a summary`);
    assert.ok(
      template.roles.length >= 2,
      `${template.id} is not a team with ${template.roles.length} role(s)`,
    );
  }
});

test("template ids are unique, so one cannot shadow another", () => {
  const ids = TEAM_TEMPLATES.map((template) => template.id);
  assert.equal(new Set(ids).size, ids.length);
});

test("role names are unique within a template", () => {
  for (const template of TEAM_TEMPLATES) {
    const names = template.roles.map((role) => role.name);
    assert.equal(
      new Set(names).size,
      names.length,
      `${template.id} repeats a role name`,
    );
  }
});

test("every role carries a prompt and a one-line description", () => {
  for (const template of TEAM_TEMPLATES) {
    for (const role of template.roles) {
      assert.ok(
        role.systemPrompt.length > 80,
        `${template.id}/${role.name} has no real prompt`,
      );
      assert.ok(
        role.description,
        `${template.id}/${role.name} has no description`,
      );
    }
  }
});

test("every team has at least one role its work depends on", () => {
  for (const template of TEAM_TEMPLATES) {
    assert.ok(
      template.roles.some((role) => role.deep),
      `${template.id} has nobody leading it`,
    );
  }
});

test("every team has at least one execution role, so it is not all leads", () => {
  for (const template of TEAM_TEMPLATES) {
    assert.ok(
      template.roles.some((role) => !role.deep),
      `${template.id} is all leads and nobody doing the work`,
    );
  }
});

test("every role inherits the shared rules, so a template behaves like the team", () => {
  for (const template of TEAM_TEMPLATES) {
    for (const role of template.roles) {
      assert.match(
        role.systemPrompt,
        /Never invent/,
        `${template.id}/${role.name} is missing the evidence rule`,
      );
      assert.match(
        role.systemPrompt,
        /Finish your turn and stop/,
        `${template.id}/${role.name} is missing the efficiency rule`,
      );
    }
  }
});

test("an unknown template id resolves to nothing rather than a default", () => {
  assert.equal(templateById("not-a-template"), undefined);
});

test("with no existing agents every role is created", () => {
  const plan = planTemplate(caseStudy, []);
  assert.equal(plan.create.length, caseStudy.roles.length);
  assert.equal(plan.skip.length, 0);
});

test("a name that already exists is skipped, never overwritten", () => {
  const plan = planTemplate(caseStudy, ["Study Lead"]);
  assert.equal(plan.skip.length, 1);
  assert.equal(plan.skip[0].name, "Study Lead");
  assert.ok(
    !plan.create.some((role) => role.name === "Study Lead"),
    "an existing agent must never be recreated",
  );
});

test("the existing-name check ignores case and surrounding space", () => {
  const plan = planTemplate(caseStudy, ["  study lead  "]);
  assert.equal(plan.skip.length, 1, "a near-miss must still count as taken");
});

test("applying a template twice does nothing the second time", () => {
  const first = planTemplate(caseStudy, []);
  const names = first.create.map((role) => role.name);
  const second = planTemplate(caseStudy, names);
  assert.equal(second.create.length, 0);
  assert.equal(second.skip.length, caseStudy.roles.length);
});

test("unrelated existing agents do not block a template", () => {
  const plan = planTemplate(caseStudy, ["Pilot Project Manager", "CORTEX"]);
  assert.equal(plan.create.length, caseStudy.roles.length);
});

test("a plan always accounts for every role in the template", () => {
  for (const template of TEAM_TEMPLATES) {
    const plan = planTemplate(template, [template.roles[0].name]);
    assert.equal(
      plan.create.length + plan.skip.length,
      template.roles.length,
      `${template.id} lost a role between create and skip`,
    );
  }
});
