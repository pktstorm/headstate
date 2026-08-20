import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { MergedDetail } from "@/types/pr";
import { InsightCards } from "./InsightCards";

const detail: MergedDetail = {
  cycle_time_hours: [0.5, 1.0, 2.0, 4.0],
  additions: 50000,
  deletions: 9139,
  changed_files: 400,
  review_count: 50,
  comment_count: 120,
  sample_size: 100,
  repo_counts: [],
};

describe("InsightCards", () => {
  // Nearest-rank on 4 samples: p50 -> index 2 (2.0), p90 -> index 3 (4.0).
  it("shows median and p90 cycle time", () => {
    render(<InsightCards detail={detail} />);
    expect(screen.getByText("2.0h")).toBeTruthy();
    expect(screen.getByText(/p90 4\.0h/)).toBeTruthy();
  });

  it("shows total lines changed", () => {
    render(<InsightCards detail={detail} />);
    expect(screen.getByText("59,139")).toBeTruthy();
  });

  it("shows review burden per PR", () => {
    render(<InsightCards detail={detail} />);
    expect(screen.getByText("1.2")).toBeTruthy(); // 120 comments / 100 PRs
  });

  // sample_size 0 would divide by zero in every per-PR figure.
  it("renders dashes rather than NaN with no sample", () => {
    render(
      <InsightCards
        detail={{ ...detail, sample_size: 0, cycle_time_hours: [] }}
      />,
    );
    expect(screen.queryByText(/NaN/)).toBeNull();
  });
});
