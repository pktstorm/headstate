/// Every help topic, in one place.
///
/// Content as DATA rather than components, for two reasons. The copy is
/// reviewable as prose without reading JSX around it, and a topic id
/// that does not exist becomes a compile error at the call site rather
/// than an empty Sheet the user opens and closes.
///
/// Markdown because the app already renders it, sanitized, for pull
/// request bodies -- so a table or a list here costs nothing new.
///
/// Written as "what this means for you", NOT copied from the source
/// comments that explain the same rules. A comment says "we do this
/// because X"; help says "this means X". The reasoning behind these
/// rules is in `derive.ts`, `scan.rs` and `docker.ts`, and translating
/// it is the work rather than a formality.
export interface HelpTopic {
  /// Shown as the Sheet's heading. A noun phrase, not a question --
  /// "Worktree safety", not "What is worktree safety?".
  title: string;
  body: string;
}

export const HELP_TOPICS = {
  "needs-attention": {
    title: "What needs your attention",
    body: `A pull request lands here when **it is blocked on you** and nobody else:

- it has **merge conflicts**, or
- its **checks are failing**

Nothing else qualifies, deliberately:

- **Checks still running** do not count — including a re-run after you
  push a fix. A failure that is being re-tested is not a failure yet,
  and nagging you through your own fix is how a list stops being worth
  reading.
- **A missing review** does not count. That is somebody else's move,
  and it appears under *waiting on others* instead.

A conflicted pull request is yours even when it is a draft. You broke
it, draft or not.`,
  },

  "triage-chips": {
    title: "The triage chips",
    body: `These are **filters, not a tally**. Clicking one narrows the list; the
numbers are not meant to add up to the total, and a pull request can
appear under more than one.

*Ready to queue* and *Awaiting review* overlap by design — a pull
request can be both.

**"of N open"** counts every open pull request in view, across the ones
you authored and the ones awaiting your review. The two counts beside
it are subsets of that, chosen by the rules above, so the arithmetic
will not close. That is the intent rather than a defect: a pull request
that is neither blocked on you nor waiting on anyone — a draft, or one
already in the merge queue — belongs in neither.`,
  },

  "pending-reviewers": {
    title: "Who you are waiting on",
    body: `The reviewers who have been **asked and have not yet answered**.

Anyone who has already reviewed is subtracted, so this is who is
outstanding rather than who was ever involved. A comment counts as an
answer: they looked and said something without blocking.

**Seeing nothing here is normal.** It means one of:

- no reviewer has been requested yet
- the repository assigns reviewers instead of requesting reviews — some
  large projects use a bot for this, and then the assignee is shown
- everyone asked has already answered

Silence is not a failure to look. It is the app saying there is nobody
specific to chase.`,
  },
} as const satisfies Record<string, HelpTopic>;

/// Every valid topic id. A call site passing anything else fails to
/// compile, which is the point of keeping this a literal record.
export type HelpTopicId = keyof typeof HELP_TOPICS;
