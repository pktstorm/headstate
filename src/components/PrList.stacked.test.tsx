import { renderWithQuery as render } from "@/test-utils";
import { cleanup, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { PR_FIXTURES } from "@/fixtures/prs";
import type { PullRequest } from "@/types/pr";
import { PrList } from "@/components/PrList";
import { useFilters } from "@/store/filters";

afterEach(() => {
  cleanup();
  useFilters.setState({ density: "comfortable" });
});

function pr(over: Partial<PullRequest>): PullRequest {
  return { ...PR_FIXTURES[0], ...over };
}

/// #743. A PR sitting on another PR rendered identically to a standalone
/// one, so nothing warned a user before they clicked "add to merge
/// queue" -- GitHub then refused it, because a stack has to be enqueued
/// through the asynchronous merge REST API instead. Four such refusals
/// across two repositories in one session log.
///
/// Tested through the LIST rather than the row, because the list is where
/// the fact becomes knowable: a row holds one PR and a stack is a
/// relationship between two. `PrRow` takes the answer as a prop and
/// cannot derive it, so a row-level test would only assert that a prop
/// renders -- it would pass with detection wired to nothing.
describe("stacked pull requests in the list", () => {
  const bottom = pr({
    id: "PR_bottom",
    number: 101,
    title: "Extract the parser",
    head_ref: "parser-extract",
    base_ref: "main",
  });
  const top = pr({
    id: "PR_top",
    number: 102,
    title: "Use the extracted parser",
    head_ref: "parser-adopt",
    base_ref: "parser-extract",
  });

  it("names the PR a stacked one sits on", () => {
    render(<PrList prs={[bottom, top]} />);
    expect(screen.getByText("on #101")).toBeTruthy();
  });

  it("says nothing on the PR at the bottom of the stack", () => {
    render(<PrList prs={[bottom, top]} />);
    expect(screen.queryByText("on #102")).toBeNull();
  });

  /// The marker has to survive dense mode. The old one -- a purple tint
  /// and a "(stacked)" suffix -- lived on the muted metadata line, and
  /// dense mode drops that line wholesale, so on a dense list the app
  /// disclosed nothing at all. That is the density a user reaching for
  /// bulk merge-queue actions is most likely to be in.
  it("still marks the stack in dense mode", () => {
    useFilters.setState({ density: "dense" });
    render(<PrList prs={[bottom, top]} />);
    expect(screen.getByText("on #101")).toBeTruthy();
  });

  /// The structural rule, stated as a test: two PRs meeting head-to-base
  /// is what makes a stack, not a branch name. `gh stack`, Graphite and
  /// `spr` each name branches their own way and a hand-made stack names
  /// them no way at all, so anything keyed on one tool's convention
  /// would recognise a quarter of the stacks in the wild.
  it("marks a stack whose branches match no tool's naming convention", () => {
    render(
      <PrList
        prs={[
          pr({ id: "PR_x", number: 201, head_ref: "tuesday", base_ref: "main" }),
          pr({ id: "PR_y", number: 202, head_ref: "wednesday", base_ref: "tuesday" }),
        ]}
      />,
    );
    expect(screen.getByText("on #201")).toBeTruthy();
  });

  /// The inverse, and the reason detection cannot simply ask whether the
  /// base is the default branch: the list does not fetch default
  /// branches, and plenty of legitimate bases are not one. A hotfix onto
  /// a release train merges the moment it is approved; marking it
  /// stacked would tell the user to go find a parent that does not exist.
  it("leaves a PR targeting a release branch unmarked", () => {
    render(
      <PrList
        prs={[pr({ id: "PR_r", number: 301, head_ref: "hotfix", base_ref: "release/2026-09" })]}
      />,
    );
    expect(screen.queryByText(/^on #/)).toBeNull();
  });

  /// Both facts, side by side. A stacked PR is routinely also a draft or
  /// blocked, and the state chip answers "what do I do about this one"
  /// while the stack chip answers "can I do anything about it yet" --
  /// suppressing either would lose an answer the row already had.
  it("shows the stack marker alongside the state chip, not instead of it", () => {
    render(<PrList prs={[bottom, { ...top, is_draft: true, merge_status: "draft" }]} />);
    expect(screen.getByText("on #101")).toBeTruthy();
    expect(screen.getAllByText(/draft/i).length).toBeGreaterThan(0);
  });
});
