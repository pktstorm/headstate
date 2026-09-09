import {
  Activity,
  BatteryCharging,
  Cpu,
  Gauge,
  HardDrive,
  MemoryStick,
  MonitorCog,
  Network,
} from "lucide-react";
import { type HealthPage, type View, useFilters } from "../store/filters";
import { ViewSwitcher } from "./ViewSwitcher";
import { useSystemHealth } from "@/api/hooks";

/// Every System Health page, in sidebar order, with its label and icon.
///
/// Exported because the PHONE renders the same list as a row of tappable
/// cards on the overview -- there is no persistent sidebar at that width
/// (see `SystemHealthPage`), so the class list has to appear somewhere
/// the thumb can reach. One array so the two cannot drift into offering
/// different pages, which is the failure #675 fixed for `VIEWS`.
///
/// The `blurb` is for the phone only: a sidebar row is read beside the
/// page it opens, so its label is enough, but a card on the overview is
/// read BEFORE anything is open and has to say what is behind it.
export const HEALTH_PAGES: {
  id: HealthPage;
  label: string;
  blurb: string;
  Icon: typeof Cpu;
}[] = [
  {
    id: "overview",
    label: "Overview",
    blurb: "Everything at a glance",
    Icon: Gauge,
  },
  {
    id: "cpu",
    label: "CPU",
    blurb: "Per-core, load, and what is using it",
    Icon: Cpu,
  },
  {
    id: "memory",
    label: "Memory",
    blurb: "Pressure, swap, and what is holding it",
    Icon: MemoryStick,
  },
  { id: "disk", label: "Disk", blurb: "Every volume, and Headstate's share", Icon: HardDrive },
  {
    id: "gpu",
    label: "GPU",
    blurb: "Utilization, memory, and the pipeline stages",
    Icon: MonitorCog,
  },
  { id: "network", label: "Network", blurb: "Every interface, since boot", Icon: Network },
  {
    id: "power",
    label: "Power and uptime",
    blurb: "Battery, thermal pressure, uptime",
    Icon: BatteryCharging,
  },
];

/// The pages worth offering on THIS machine.
///
/// Every page in `HEALTH_PAGES` but GPU is offered unconditionally: a
/// machine always has a CPU, memory, volumes, interfaces and an uptime,
/// and where one of those cannot be read the page says "Not measured",
/// which is informative.
///
/// GPU is the exception, and it is the same rule #705 set for the
/// overview panel. An empty GPU page would claim we found a graphics
/// device and could not read it -- which on Windows and on Intel or
/// NVIDIA Linux is not what happened: we did not look, because there is
/// no unprivileged way to. So a machine with no discoverable GPU is
/// offered no GPU page at all, exactly as #717 asks.
///
/// Derived from the live sample rather than from a capability flag, for
/// the same reason the panel is: `gpus` being empty IS the fact, and a
/// second source for it could disagree with the first.
///
/// Exported so the phone's card list and the sidebar cannot drift into
/// offering different pages -- the failure `HEALTH_PAGES` itself exists
/// to prevent.
export function healthPagesFor(gpuCount: number) {
  return HEALTH_PAGES.filter((p) => p.id !== "gpu" || gpuCount > 0);
}

/// The System Health view's own sidebar (#687).
///
/// # Why this column exists at all
///
/// The view deliberately has no repository list: #664 established that
/// it describes the MACHINE, so a repo picker beside it would be a
/// control that changes nothing. That left the column holding only the
/// view switcher. The class list is the natural occupant -- it is
/// navigation within the thing the page is about, which is exactly what
/// every other sidebar in the app holds.
///
/// # Overview is a row, not a back button
///
/// It sits at the top of the same list as the others rather than being
/// reachable only by a "back" affordance, because it is a peer: the
/// landing page, and the one that answers "is anything wrong" fastest.
/// A back button would make it feel like somewhere you leave, when it is
/// the place people arrive.
///
/// Desktop only. At phone width `App` shows no persistent sidebar, and
/// this list appears on the overview itself instead -- see
/// `SystemHealthPage`, which is where that decision is written down.
export function SystemHealthSidebar({
  viewCounts,
}: {
  viewCounts?: Partial<Record<View, number>>;
}) {
  const { healthPage, setHealthPage } = useFilters();
  // `false`: this reads the cache that `SystemHealthPage` -- always
  // mounted beside this column -- is already filling. A second enabled
  // observer here would be a second poller on the same key, and the
  // sidebar has no reason to drive a measurement of its own. Before the
  // first sample lands there is no GPU to offer, which is correct: the
  // row appears when the machine has answered, not before.
  const gpuCount = useSystemHealth(false).data?.gpus.length ?? 0;
  const pages = healthPagesFor(gpuCount);

  return (
    <nav className="flex w-64 shrink-0 flex-col border-r border-[#30363d] p-3">
      <ViewSwitcher counts={viewCounts} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* A heading, because these rows are a different KIND of thing
            from the switcher above them: that one changes which view you
            are in, these move within one view. Two unlabelled stacks of
            buttons would read as one list where the first entry happens
            to look different. */}
        <div className="px-3 pb-1 pt-2 text-xs font-semibold uppercase tracking-wide text-[#8b949e]">
          <Activity className="mr-1.5 inline h-3 w-3" aria-hidden="true" />
          This machine
        </div>
        <ul className="flex flex-col">
          {pages.map(({ id, label, Icon }) => (
            <li key={id}>
              <button
                type="button"
                onClick={() => setHealthPage(id)}
                // `aria-current="page"` rather than `aria-pressed`: these
                // are navigation, not toggles, and a screen reader
                // announcing "pressed" for the page you are already on
                // describes a control that did something rather than a
                // location you are at.
                aria-current={id === healthPage ? "page" : undefined}
                className={`flex w-full items-center gap-2 rounded px-3 py-2 text-sm ${
                  id === healthPage
                    ? "bg-[#1f6feb] text-white"
                    : "text-[#e6edf3] hover:bg-[#161b22]"
                }`}
              >
                <Icon className="h-4 w-4 shrink-0" aria-hidden="true" />
                <span className="truncate">{label}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </nav>
  );
}
