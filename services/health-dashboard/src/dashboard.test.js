import assert from "node:assert/strict";
import test from "node:test";
import { compareThreshold, formatError, formatMetric, formatTimestamp, healthColor } from "./dashboard.js";

test("formats millions", () => assert.equal(formatMetric(1_250_000), "1.25M"));
test("formats thousands", () => assert.equal(formatMetric(12_500), "12.5K"));
test("formats small metrics", () => assert.equal(formatMetric(12.5), "12.50"));
test("handles non-numeric metrics", () => assert.equal(formatMetric("not-a-number"), "—"));
test("formats timestamps", () => assert.notEqual(formatTimestamp(Date.now()), "—"));
test("compares above thresholds", () => assert.equal(compareThreshold(11, 10), true));
test("compares below thresholds", () => assert.equal(compareThreshold(9, 10, "below"), true));
test("rejects invalid threshold values", () => assert.equal(compareThreshold("x", 10), false));
test("selects health colors", () => { assert.equal(healthColor(80), "var(--green)"); assert.equal(healthColor(50), "var(--yellow)"); assert.equal(healthColor(10), "var(--red)"); });
test("formats invocation errors", () => assert.equal(formatError(new Error("offline")), "offline"));
