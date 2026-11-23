import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import Ajv2020 from "ajv/dist/2020.js";

const load = (name) => JSON.parse(readFileSync(new URL(name, import.meta.url), "utf8"));
const tools = load("tools.json");
const examples = load("examples.json");
const ajv = new Ajv2020({ allowUnionTypes: true });

for (const { name, inputSchema, resultSchema } of tools) {
  test(name, () => {
    const accepts = ajv.compile(inputSchema);
    const result = ajv.compile(resultSchema);
    const example = examples[name];
    const args = example.arguments;
    assert.equal(accepts(args), true);
    assert.equal(result(example.result), true);
    for (const wrong of [null, [], "text", 0]) assert.equal(accepts(wrong), false);
    for (const field of ["bindingId", "projectId", "cwd", "host", "port", "connectionString", "headers", "limit"]) {
      assert.equal(accepts({ ...args, [field]: "untrusted" }), false, field);
    }
    for (const field of inputSchema.required) {
      const missing = { ...args };
      delete missing[field];
      assert.equal(accepts(missing), false, field);
    }
    for (const [field, rule] of Object.entries(inputSchema.properties)) {
      const acceptsValue = (value) => accepts({ ...args, [field]: value });
      for (const value of ["", null, 7, "a".repeat(rule.maxLength + 1)]) {
        assert.equal(acceptsValue(value), false, field);
      }
      assert.equal(acceptsValue("a".repeat(rule.maxLength)), true);
      if (field === "sql") {
        assert.equal(acceptsValue(" \t\n"), false);
        assert.equal(acceptsValue("列".repeat(rule.maxLength)), true);
      } else {
        for (const value of ["/tmp/orders", "C:\\work", "../other", "mysql://db:3306", "10.0.0.1", "db.example", "bad\n"]) {
          assert.equal(acceptsValue(value), false, value);
        }
      }
    }
    assert.equal(result({ ...example.result, host: "private.invalid" }), false);
    if (name === "environment_list") {
      const invalid = structuredClone(example.result);
      invalid.environments[0].resources[0].health = "invented";
      assert.equal(result(invalid), false);
    }
    if (name === "database_schema") assert.equal(accepts({target: "dev", resource: "primary-db"}), true);
    if (name === "database_query") assert.equal(result({ ...example.result, rows: Array(201).fill([1]) }), false);
  });
}

test("CLI snapshots conform to the common response schema", () => {
  const validate = ajv.compile(load("report.schema.json"));
  for (const { json } of load("cli.json")) {
    assert.equal(validate(json), true, JSON.stringify(validate.errors));
    for (const field of ["schemaVersion", "requestId", "timestampUtc", json.ok ? "evidenceId" : "errorCode"]) {
      const incomplete = { ...json };
      delete incomplete[field];
      assert.equal(validate(incomplete), false, field);
    }
    assert.equal(validate({ ...json, ok: !json.ok }), false);
    assert.equal(validate({ ...json, status: "pending" }), false);
    assert.equal(validate({ ...json, timestampUtc: "2026-11-23T16:00:00+08:00" }), false);
  }
});
