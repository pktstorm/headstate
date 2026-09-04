import { describe, expect, it } from "vitest";
import { prFromUrl } from "./hooks";

/// The toast's "Open" action only appears when this parses, so a shape
/// we do not recognise costs a button rather than producing one that
/// goes nowhere.
describe("prFromUrl", () => {
  it("reads owner/repo and number from a pull request URL", () => {
    expect(prFromUrl("https://github.com/octocat/hello-world/pull/42")).toEqual({
      repo: "octocat/hello-world",
      number: 42,
    });
  });

  it("handles a URL with a trailing path or query", () => {
    expect(prFromUrl("https://github.com/octocat/hello-world/pull/7/files")).toEqual({
      repo: "octocat/hello-world",
      number: 7,
    });
  });

  it("returns null rather than guessing at anything else", () => {
    expect(prFromUrl("https://github.com/octocat/hello-world")).toBeNull();
    expect(prFromUrl("https://example.invalid/octocat/hello-world/pull/1")).toBeNull();
    expect(prFromUrl("")).toBeNull();
    expect(prFromUrl("not a url")).toBeNull();
  });

  /// An issue is not a pull request, and opening the wrong thing is
  /// worse than opening nothing.
  it("does not match an issue URL", () => {
    expect(prFromUrl("https://github.com/octocat/hello-world/issues/42")).toBeNull();
  });
});
