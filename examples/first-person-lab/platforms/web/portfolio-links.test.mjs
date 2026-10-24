import assert from "node:assert/strict";
import test from "node:test";
import { isPortfolioInteraction, openPortfolioLink } from "./portfolio-links.ts";

test("only portfolio halves own link interactions", () => {
  for (const target of [4, 5]) assert.equal(isPortfolioInteraction(target), true);
  for (const target of [0, 1, 3, 6, 100, 200, NaN]) assert.equal(isPortfolioInteraction(target), false);
});

test("GitHub opens synchronously in an isolated new tab", () => {
  const calls = [];
  assert.equal(openPortfolioLink({ open: (...args) => calls.push(args) }, 4), true);
  assert.deepEqual(calls, [["https://github.com/jdharrison", "_blank", "noopener,noreferrer"]]);
});

test("LinkedIn opens the supplied profile synchronously in an isolated new tab", () => {
  const calls = [];
  assert.equal(openPortfolioLink({ open: (...args) => calls.push(args) }, 5), true);
  assert.deepEqual(calls, [["https://www.linkedin.com/in/jacobdharrison", "_blank", "noopener,noreferrer"]]);
});

test("unrelated targets never open placeholder URLs", () => {
  const child = { open: () => assert.fail("unexpected navigation") };
  for (const target of [0, 1, 100, 200]) assert.equal(openPortfolioLink(child, target), false);
});
