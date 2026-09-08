import { describe, expect, it } from "vitest";
import { summarisePull } from "./pullSummary";

/// Real `git pull --ff-only` output, not invented shapes.
const FAST_FORWARD = `Updating a1b2c3d..e4f5a6b
Fast-forward
 src/a.ts   |   2 +-
 src/b.ts   |  40 ++++++++++++++
 2 files changed, 41 insertions(+), 1 deletion(-)
`;

const BIG = `Updating 1111111..2222222
Fast-forward
${Array.from({ length: 400 }, (_, i) => ` src/file${i}.ts | 3 ++-`).join("\n")}
 400 files changed, 1200 insertions(+), 400 deletions(-)
`;

describe("summarisePull", () => {
  it("reduces a fast-forward to its range and diffstat", () => {
    expect(summarisePull(FAST_FORWARD)).toBe(
      "a1b2c3d..e4f5a6b — 2 files changed, 41 insertions(+), 1 deletion(-)",
    );
  });

  it("stays one line when the diffstat is hundreds", () => {
    // The reported bug: a real repo produced hundreds of lines and the
    // toast showed all of them.
    const out = summarisePull(BIG);
    expect(out.split("\n")).toHaveLength(1);
    expect(out).toContain("400 files changed");
  });

  it("keeps git's own words when nothing was fetched", () => {
    // Must NOT become "Updated": nothing changed, and saying it did
    // would be a small lie the previous code was careful to avoid.
    expect(summarisePull("Already up to date.\n")).toBe("Already up to date.");
  });

  it("survives git's older wording and a translated locale", () => {
    // Decided from the diffstat's absence, not by matching prose, so a
    // reworded or translated sentence still reads as "nothing fetched".
    expect(summarisePull("Already up-to-date.\n")).toBe("Already up-to-date.");
    expect(summarisePull("Déjà à jour.\n")).toBe("Déjà à jour.");
  });

  it("handles a single-file change without mangling the plural", () => {
    const one = `Updating aaa..bbb
Fast-forward
 README.md | 1 +
 1 file changed, 1 insertion(+)
`;
    expect(summarisePull(one)).toBe("aaa..bbb — 1 file changed, 1 insertion(+)");
  });

  it("falls back to something rather than an empty toast", () => {
    expect(summarisePull("")).toBe("Already up to date");
    expect(summarisePull("   \n  ")).toBe("Already up to date");
  });
});
