import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Outdated, RunReport } from "@/types/pr";

const applyFn = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<RunReport>>());
const prFn = vi.hoisted(() => vi.fn<(...a: unknown[]) => Promise<string>>());
const toasts = vi.hoisted(() => ({
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => ({ toast: toasts }));
vi.mock("../api/tauri", () => ({ applyUpdatesInBackground: applyFn }));

import { UpdateWizard } from "./UpdateWizard";

const pkg = (name: string, ecosystem: Outdated["ecosystem"] = "npm"): Outdated => ({
  name,
  current: "1.0.0",
  latest: "2.0.0",
  bump: "major",
  ecosystem,
  manifest: "package.json",
});

const show = (packages: Outdated[]) =>
  render(
    <UpdateWizard repo="/code/app" packages={packages} open onOpenChange={vi.fn()} />,
  );

describe("UpdateWizard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    prFn.mockResolvedValue("https://github.com/octocat/hello-world/pull/7");
    applyFn.mockResolvedValue({
      worktree: "/code/app/.worktrees/update-lodash",
      branch: "headstate/update-lodash",
      ecosystems: ["npm"],
      results: [
        {
          name: "lodash",
          requested: "4.17.21",
          changed_files: ["package.json", "package-lock.json"],
          output: "",
          resolved_constraint: "^4.17.21",
          error: null,
        },
      ],
    });
  });

  it("says plainly that nothing is pushed", () => {
    show([pkg("lodash")]);
    expect(screen.getByText(/Nothing is pushed/i)).toBeTruthy();
  });

  /// Swift cannot be applied, and saying so beats omitting it: a package
  /// that silently cannot be selected reads as a bug in the list.
  it("lists unappliable packages with a reason instead of hiding them", () => {
    show([pkg("lodash"), pkg("Alamofire", "swift")]);
    expect(screen.getByText("Alamofire")).toBeTruthy();
    expect(screen.getByText(/Xcode/)).toBeTruthy();
    // And it is not selectable.
    expect(screen.getAllByRole("checkbox")).toHaveLength(1);
  });

  it("does not apply until something is selected", () => {
    show([pkg("lodash")]);
    const button = screen.getByRole("button", { name: /Apply/ });
    expect(button.hasAttribute("disabled")).toBe(true);
    fireEvent.click(button);
    expect(applyFn).not.toHaveBeenCalled();
  });







  /// #478: one repository declares the same dependency in several
  /// manifests, so `ecosystem:name` was not a unique row identity --
  /// ticking one checkbox ticked every row with that name.
  it("gives each manifest its own checkbox for the same package", () => {
    const dup = (manifest: string, current: string): Outdated => ({
      ...pkg("registry.terraform.io/hashicorp/aws"),
      manifest,
      current,
    });
    show([dup("a/main.tf", "5.100.0"), dup("b/main.tf", "5.90.0")]);

    const boxes = screen.getAllByRole("checkbox");
    expect(boxes).toHaveLength(2);
    fireEvent.click(boxes[0]);
    expect((boxes[0] as HTMLInputElement).checked).toBe(true);
    expect((boxes[1] as HTMLInputElement).checked).toBe(false);
  });

  /// Two rows for one dependency are indistinguishable without saying
  /// which file each came from.
  it("names the manifest each row belongs to", () => {
    show([pkg("lodash")]);
    expect(screen.getByText("package.json")).toBeTruthy();
  });

  it("selects and clears every applicable package", () => {
    show([pkg("lodash"), pkg("express")]);
    fireEvent.click(screen.getByRole("button", { name: /select all 2/i }));
    for (const b of screen.getAllByRole("checkbox")) {
      expect((b as HTMLInputElement).checked).toBe(true);
    }
    fireEvent.click(screen.getByRole("button", { name: /^clear$/i }));
    for (const b of screen.getAllByRole("checkbox")) {
      expect((b as HTMLInputElement).checked).toBe(false);
    }
  });

  /// The backend refuses Terraform (`apply::supported`), but the UI
  /// listed it as applicable -- so every Terraform row was selectable,
  /// counted toward the button, and failed at apply time with a reason
  /// the user could have been given before clicking.
  it("does not offer Terraform providers as applicable", () => {
    show([pkg("registry.terraform.io/hashicorp/aws", "terraform")]);
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
    expect(screen.getByText(/constraint in your .tf source/i)).toBeTruthy();
  });

  /// The `2.8.0 → 2.8.0` rows in the report. `registry::enrich` leaves
  /// a provider at `latest == current` with `bump: "unknown"` when its
  /// lookup FAILS -- meaning "cannot compare", not "up to date". The
  /// wizard rendered that identically to a real update.
  it("does not offer a row whose latest could not be determined", () => {
    const unknown: Outdated = {
      ...pkg("registry.terraform.io/hashicorp/archive"),
      ecosystem: "npm",
      current: "2.8.0",
      latest: "2.8.0",
      bump: "unknown",
    };
    show([unknown]);
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
    expect(screen.getByText(/could not be determined/i)).toBeTruthy();
  });


  /// #495: the run does not need the dialog.
  ///
  /// Apply used to await the whole run with the modal open on one
  /// unchanging "Applying…" -- minutes on a large selection, with the
  /// app unusable throughout. The outcome now arrives on an event.
  it("starts the run and closes immediately", async () => {
    const onOpenChange = vi.fn();
    render(
      <UpdateWizard repo="/code/app" packages={[pkg("lodash")]} open onOpenChange={onOpenChange} />,
    );
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /^apply/i }));

    await waitFor(() => expect(applyFn).toHaveBeenCalledTimes(1));
    expect(applyFn).toHaveBeenCalledWith("/code/app", [
      { name: "lodash", version: "2.0.0", ecosystem: "npm" },
    ]);
    // Closed on click, not on completion.
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("says the run has started rather than closing silently", async () => {
    show([pkg("lodash")]);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /^apply/i }));

    await waitFor(() =>
      expect(toasts.info).toHaveBeenCalledWith(
        expect.stringMatching(/updating 1 package/i),
        expect.anything(),
      ),
    );
  });

  /// A run that could not even START is a different thing from one that
  /// ran and reported failures, and must not be silent.
  it("reports a run that could not be started", async () => {
    applyFn.mockRejectedValue("no such repository");
    show([pkg("lodash")]);
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /^apply/i }));

    await waitFor(() =>
      expect(toasts.error).toHaveBeenCalledWith("Could not start the update run", {
        description: "no such repository",
      }),
    );
  });
});
