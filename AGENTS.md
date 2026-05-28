# Agent Instructions

## Plan Implementation Tracking

When implementing a plan from the `plans/` folder, first check whether the plan file includes a todo table for tracking progress. If it does not, add one with a checkbox for each todo item before starting implementation.

After implementation and testing, update the same plan file: check off completed todo items and mark them as completed.

## Git Commit Format

Commit messages should use a concise Conventional Commit style subject, followed by an explanatory body when the change is non-trivial:

```text
<type>: <short imperative summary>

Explain what changed and why. Prefer behavior and architecture context over a file-by-file list.
Keep related details together in short paragraphs.
```

Use types such as `feat`, `fix`, `refactor`, `docs`, `test`, and `chore`. Keep the subject lowercase after the type, omit a trailing period, and leave a blank line between the subject and body.
